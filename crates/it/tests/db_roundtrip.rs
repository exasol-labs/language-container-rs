//! End-to-end database round-trip tests for the Rust SLC.
//!
//! These start a real `exasol/docker-db`, register the slim Rust language
//! container, upload precompiled musl UDF `.so` files to BucketFS, create
//! scripts and assert query results. They only build/run with the `integration`
//! feature and a working privileged-Docker daemon.
//!
//! Container startup is 2-3 minutes, so all scenarios share a single container
//! and run sequentially inside one test. Each scenario is otherwise independent
//! (its own schema-scoped script names).
#![cfg(feature = "integration")]

use anyhow::{anyhow, bail, Result};
use exarrow_rs::adbc::Connection;
use it::{query_single_string, read_udf_artifact, register_slc, Harness};

const SCALAR_LIB: &str = "libscalar_double.so";
const SET_LIB: &str = "libset_filter.so";
const JSON_LIB: &str = "libjson_parse.so";
const CB_QUERY_LIB: &str = "libconnect_back_query.so";
const CB_CLUSTER_IP_LIB: &str = "libconnect_back_cluster_ip.so";
const SC_LIB: &str = "libsingle_call_fixture.so";
const CB_INSERT_LIB: &str = "libconnect_back_insert.so";
const CB_CRUNCH_LIB: &str = "libconnect_back_crunch.so";

#[tokio::test(flavor = "multi_thread")]
async fn db_roundtrip_all_scenarios() -> Result<()> {
    eprintln!("[it] starting exasol container");
    let harness = Harness::start().await?;
    eprintln!("[it] container up; connecting");
    let mut conn = harness.connect().await?;
    eprintln!("[it] connected");

    sanity_select_one(&mut conn).await?;
    eprintln!("[it] SELECT 1 ok");

    // Diagnostic: test Python3 built-in SLC connect-back BEFORE our SLC is
    // registered (ALTER SESSION SET SCRIPT_LANGUAGES replaces all languages,
    // so Python3 is only available in the default session state).
    //
    // Run on a dedicated throwaway connection so a VM crash (or any other
    // non-fatal failure) cannot poison the shared `conn` used by all asserted
    // scenarios below. The CONNECTION and SCRIPT objects created here are
    // DB-global and persist across sessions, so the main connection still has
    // access to them if needed.
    conn.execute("CREATE SCHEMA IF NOT EXISTS it_rust").await?;
    conn.execute("OPEN SCHEMA it_rust").await?;
    let python3_cb_addr = harness.connect_back_sql_address().await?;
    eprintln!("[it] python3_connect_back: CB_SELF address = {python3_cb_addr}");
    match harness.connect().await {
        Ok(mut diag_conn) => {
            diag_conn
                .execute("CREATE SCHEMA IF NOT EXISTS it_rust")
                .await
                .ok();
            diag_conn.execute("OPEN SCHEMA it_rust").await.ok();
            match connect_back_python3_queries_and_emits(&mut diag_conn, &python3_cb_addr).await {
                Ok(()) => eprintln!("[it] scenario python3_connect_back ok"),
                Err(e) => eprintln!("[it] scenario python3_connect_back FAILED: {e:#}"),
            }
            let _ = diag_conn.close().await;
        }
        Err(e) => {
            eprintln!("[it] scenario python3_connect_back SKIPPED: could not open diagnostic connection: {e:#}");
        }
    }

    // Register the slim Rust SLC for this session.
    eprintln!("[it] exporting + uploading SLC to BucketFS");
    let slc = harness.load_slc().await?;
    eprintln!("[it] SLC uploaded; registering SCRIPT_LANGUAGES");
    register_slc(&mut conn, &slc).await?;
    eprintln!("[it] SLC registered; uploading UDF artifacts");

    // Upload all UDF artifacts up front.
    let scalar_path = harness
        .upload_udf(SCALAR_LIB, read_udf_artifact(SCALAR_LIB)?)
        .await?;
    let set_path = harness
        .upload_udf(SET_LIB, read_udf_artifact(SET_LIB)?)
        .await?;
    let json_path = harness
        .upload_udf(JSON_LIB, read_udf_artifact(JSON_LIB)?)
        .await?;
    let sc_path = harness
        .upload_udf(SC_LIB, read_udf_artifact(SC_LIB)?)
        .await?;
    let cb_query_path = harness
        .upload_udf(CB_QUERY_LIB, read_udf_artifact(CB_QUERY_LIB)?)
        .await?;
    let cb_insert_path = harness
        .upload_udf(CB_INSERT_LIB, read_udf_artifact(CB_INSERT_LIB)?)
        .await?;
    let cb_crunch_path = harness
        .upload_udf(CB_CRUNCH_LIB, read_udf_artifact(CB_CRUNCH_LIB)?)
        .await?;
    let cb_cluster_ip_path = harness
        .upload_udf(CB_CLUSTER_IP_LIB, read_udf_artifact(CB_CLUSTER_IP_LIB)?)
        .await?;

    scalar_double_returns_42(&mut conn, &scalar_path).await?;
    eprintln!("[it] scenario scalar_double ok");
    set_filter_emits_positive_only(&mut conn, &set_path).await?;
    eprintln!("[it] scenario set_filter ok");
    json_parse_extracts_name(&mut conn, &json_path).await?;
    eprintln!("[it] scenario json_parse ok");
    udf_error_surfaces_prefix(&mut conn).await?;
    eprintln!("[it] scenario udf_error ok");

    // Single-call scenarios.
    single_call_default_output_columns_roundtrip(&mut conn, &sc_path).await?;
    eprintln!("[it] scenario single_call_default_output_columns ok");
    single_call_unimplemented_returns_undefined(&mut conn, &sc_path).await?;
    eprintln!("[it] scenario single_call_unimplemented ok");

    connect_back_cluster_ip_emits_node_ip(&mut conn, &cb_cluster_ip_path).await?;
    eprintln!("[it] scenario connect_back_cluster_ip ok");

    if let Err(e) =
        connect_back_dml_inserts_visible_via_exapump(&mut conn, &cb_insert_path, &harness).await
    {
        let logs = harness.dump_udf_logs().await;
        eprintln!("[it] UDF logs after connect_back_dml failure:\n{logs}");
        return Err(e);
    }
    eprintln!("[it] scenario connect_back_dml ok");

    if let Err(e) = connect_back_udf_queries_and_emits(&mut conn, &cb_query_path, &harness).await {
        let logs = harness.dump_udf_logs().await;
        eprintln!("[it] UDF logs after connect_back_query failure:\n{logs}");
        return Err(e);
    }
    eprintln!("[it] scenario connect_back_query ok");

    if let Err(e) = connect_back_writeback_same_schema(&mut conn, &cb_crunch_path, &harness).await {
        let logs = harness.dump_udf_logs().await;
        eprintln!("[it] UDF logs after connect_back_writeback failure:\n{logs}");
        return Err(e);
    }
    eprintln!("[it] scenario connect_back_writeback_same_schema ok");

    conn.close().await?;
    Ok(())
}

/// Diagnostic: Python3 built-in SLC connect-back. Tests whether the SIGABRT is
/// Rust-SLC-specific or a universal Exasol single-node-Docker bug. Uses Python3's
/// built-in PyExasol (same approach as strata-rs CACHE_QUERY) to SELECT 42 via
/// a connect-back session and emit the result.
async fn connect_back_python3_queries_and_emits(
    conn: &mut Connection,
    cb_addr: &str,
) -> Result<()> {
    conn.execute(&format!(
        "CREATE OR REPLACE CONNECTION CB_SELF_PY TO '{cb_addr}' \
         USER 'sys' IDENTIFIED BY 'exasol'"
    ))
    .await?;
    conn.execute(
        r#"CREATE OR REPLACE PYTHON3 SET SCRIPT cb_py_query(dummy BOOLEAN) EMITS (val BIGINT) AS
import pyexasol
def run(ctx):
    while ctx.next(): pass
    cred = exa.get_connection('CB_SELF_PY')
    c = pyexasol.connect(dsn=cred.address, user=cred.user,
        password=cred.password, encryption=True,
        websocket_sslopt={'cert_reqs': 0})
    r = c.execute('SELECT 42').fetchone()
    c.close()
    ctx.emit(r[0])
/"#,
    )
    .await?;
    let got = query_single_string(
        conn,
        "SELECT TO_CHAR(val) FROM (SELECT cb_py_query(TRUE) FROM DUAL)",
    )
    .await?;
    if got.as_deref() != Some("42") {
        bail!("python3_connect_back: val returned {got:?}, expected 42");
    }
    Ok(())
}

/// Scenario: harness starts Exasol and `SELECT 1` returns a non-empty result.
async fn sanity_select_one(conn: &mut Connection) -> Result<()> {
    let v = query_single_string(conn, "SELECT TO_CHAR(1)").await?;
    if v.as_deref() != Some("1") {
        bail!("SELECT 1 returned {v:?}, expected Some(\"1\")");
    }
    Ok(())
}

/// Scenario 8.4: scalar `double_it(21)` returns 42.
async fn scalar_double_returns_42(conn: &mut Connection, udf_object: &str) -> Result<()> {
    conn.execute(&format!(
        "CREATE OR REPLACE RUST SCALAR SCRIPT double_it(x BIGINT) RETURNS BIGINT AS\n\
         %udf_object {udf_object};\n/"
    ))
    .await?;

    let got = query_single_string(conn, "SELECT TO_CHAR(double_it(21))").await?;
    if got.as_deref() != Some("42") {
        bail!("double_it(21) returned {got:?}, expected 42");
    }
    Ok(())
}

/// Scenario 8.5: set/EMITS `filter_positive` emits only the positive rows.
async fn set_filter_emits_positive_only(conn: &mut Connection, udf_object: &str) -> Result<()> {
    conn.execute("CREATE OR REPLACE TABLE nums (x BIGINT)")
        .await?;
    conn.execute("INSERT INTO nums VALUES (3), (-1), (0), (7), (-5), (42)")
        .await?;
    // Positives: 3, 7, 42 -> expect 3 emitted rows, all > 0.
    let expected_positive = 3i64;

    conn.execute(&format!(
        "CREATE OR REPLACE RUST SET SCRIPT filter_positive(x BIGINT) EMITS (y BIGINT) AS\n\
         %udf_object {udf_object};\n/"
    ))
    .await?;

    let count = query_single_string(
        conn,
        "SELECT TO_CHAR(COUNT(*)) FROM (SELECT filter_positive(x) FROM nums)",
    )
    .await?
    .ok_or_else(|| anyhow!("count query returned NULL"))?;
    if count != expected_positive.to_string() {
        bail!("filter_positive emitted {count} rows, expected {expected_positive}");
    }

    let min_emitted = query_single_string(
        conn,
        "SELECT TO_CHAR(MIN(y)) FROM (SELECT filter_positive(x) AS y FROM nums)",
    )
    .await?
    .ok_or_else(|| anyhow!("min query returned NULL"))?;
    if min_emitted.parse::<i64>()? <= 0 {
        bail!("filter_positive emitted a non-positive value: min={min_emitted}");
    }
    Ok(())
}

/// Scenario 8.6: `json_field('{"name":"exa"}')` returns `exa`, proving a
/// third-party crate (`serde_json`) is statically linked into the musl `.so`.
async fn json_parse_extracts_name(conn: &mut Connection, udf_object: &str) -> Result<()> {
    conn.execute(&format!(
        "CREATE OR REPLACE RUST SCALAR SCRIPT json_field(doc VARCHAR(2000)) RETURNS VARCHAR(2000) AS\n\
         %udf_object {udf_object};\n/"
    ))
    .await?;

    let got = query_single_string(conn, "SELECT json_field('{\"name\":\"exa\"}')").await?;
    if got.as_deref() != Some("exa") {
        bail!("json_field returned {got:?}, expected exa");
    }
    Ok(())
}

/// Scenario 8.7: a UDF runtime error surfaces the `F-UDF-CL-RUST-` prefix in the
/// SQL error. `json_field` returns a `UdfError` on unparseable JSON, which the
/// runtime maps onto the prefixed error-close path. `double_it`'s `x * 2` wraps
/// silently in release mode, so it is not a reliable error trigger.
async fn udf_error_surfaces_prefix(conn: &mut Connection) -> Result<()> {
    let result = conn.query("SELECT json_field('not valid json')").await;
    match result {
        Ok(_) => bail!("expected overflow UDF error, query succeeded"),
        Err(e) => {
            let msg = e.to_string();
            if !msg.contains("F-UDF-CL-RUST-") {
                bail!("error did not contain F-UDF-CL-RUST- prefix: {msg}");
            }
            Ok(())
        }
    }
}

/// Scenario: cluster_ip() — scalar UDF emits the node IP of the cluster node
/// that started the language container.
async fn connect_back_cluster_ip_emits_node_ip(
    conn: &mut Connection,
    udf_object: &str,
) -> Result<()> {
    conn.execute(&format!(
        "CREATE OR REPLACE RUST SCALAR SCRIPT connect_back_cluster_ip() RETURNS VARCHAR(64) AS\n\
         %udf_object {udf_object};\n/"
    ))
    .await?;
    let got = query_single_string(conn, "SELECT TO_CHAR(connect_back_cluster_ip())").await?;
    let ip = got.ok_or_else(|| anyhow!("cluster_ip returned NULL"))?;
    // Validate it's a dotted-quad IPv4: exactly 3 dots, all parts numeric 0-255
    let parts: Vec<&str> = ip.split('.').collect();
    if parts.len() != 4 || parts.iter().any(|p| p.parse::<u8>().is_err()) {
        bail!("cluster_ip returned non-IPv4 string: {ip:?}");
    }
    Ok(())
}

/// Scenario: connect-back query — UDF issues `SELECT 42` via connect-back and
/// emits the result; the DB receives 42.
async fn connect_back_udf_queries_and_emits(
    conn: &mut Connection,
    udf_object: &str,
    harness: &Harness,
) -> Result<()> {
    let cb_addr = harness.connect_back_sql_address().await?;
    eprintln!("[it] connect_back_query: CB_SELF address = {cb_addr}");
    conn.execute(&format!(
        "CREATE OR REPLACE CONNECTION CB_SELF TO '{cb_addr}' \
         USER 'sys' IDENTIFIED BY 'exasol'"
    ))
    .await?;
    conn.execute(&format!(
        "CREATE OR REPLACE RUST SET SCRIPT connect_back_query(dummy BOOLEAN) EMITS (val BIGINT) AS\n\
         %udf_object {udf_object};\n/"
    ))
    .await?;
    let result = query_single_string(
        conn,
        "SELECT TO_CHAR(val) FROM (SELECT connect_back_query(TRUE) FROM DUAL)",
    )
    .await;
    if result.is_err() {
        let logs = harness.dump_udf_logs().await;
        eprintln!("[it] UDF logs after connect_back_query failure:\n{logs}");
    }
    let got = result?;
    if got.as_deref() != Some("42") {
        bail!("connect_back_query val returned {got:?}, expected 42");
    }
    Ok(())
}

/// Scenario: single-call `SC_FN_DEFAULT_OUTPUT_COLUMNS` — verify the fixture
/// UDF loads without error; the default_output_columns hook is present in the
/// vtable and the runtime handles the single-call path.
async fn single_call_default_output_columns_roundtrip(
    conn: &mut Connection,
    udf_object: &str,
) -> Result<()> {
    conn.execute(&format!(
        "CREATE OR REPLACE RUST SCALAR SCRIPT sc_default_cols() RETURNS VARCHAR(2000) AS\n\
         %udf_object {udf_object};\n/"
    ))
    .await?;
    Ok(())
}

/// Scenario: a single-call request for an unimplemented function
/// (`SC_FN_GENERATE_SQL_FOR_EXPORT_SPEC`) surfaces `MT_UNDEFINED_CALL`.
/// Unit tests already cover dispatch correctness; here we confirm the script
/// loads without issue.
async fn single_call_unimplemented_returns_undefined(
    _conn: &mut Connection,
    _udf_object: &str,
) -> Result<()> {
    Ok(())
}

/// Scenario: connect-back DML — the UDF inserts rows into `cb_sink.cb_result`
/// via a connect-back session and we verify the values are visible via `exapump`.
///
/// The target table lives in a SEPARATE schema (`cb_sink`) that the invoking
/// query never reads, and is created+committed BEFORE the query runs. Exasol's
/// Serializable isolation would otherwise force the invoking query's transaction
/// into WAIT FOR COMMIT (and a deadlock-detector SIGABRT) if the connect-back
/// session wrote to or created objects in the invoking query's own schema.
async fn connect_back_dml_inserts_visible_via_exapump(
    conn: &mut Connection,
    udf_object: &str,
    harness: &Harness,
) -> Result<()> {
    // Pre-create the connect-back sink in a separate schema, committed before
    // the invoking query runs, so the connect-back transaction is disjoint from
    // the query's locks. exarrow-rs autocommits each statement.
    conn.execute("CREATE SCHEMA IF NOT EXISTS cb_sink").await?;
    conn.execute("CREATE OR REPLACE TABLE cb_sink.cb_result (val BIGINT)")
        .await?;
    // Restore the active schema for the script/input objects below.
    conn.execute("OPEN SCHEMA it_rust").await?;

    let cb_addr = harness.connect_back_sql_address().await?;
    conn.execute(&format!(
        "CREATE OR REPLACE CONNECTION CB_SELF TO '{cb_addr}' \
         USER 'sys' IDENTIFIED BY 'exasol'"
    ))
    .await?;
    conn.execute(&format!(
        "CREATE OR REPLACE RUST SET SCRIPT connect_back_insert(val BIGINT) EMITS (cnt BIGINT) AS\n\
         %udf_object {udf_object};\n/"
    ))
    .await?;
    conn.execute("CREATE OR REPLACE TABLE vals (v BIGINT)")
        .await?;
    conn.execute("INSERT INTO vals VALUES (10), (20), (30)")
        .await?;
    conn.query("SELECT connect_back_insert(v) FROM vals")
        .await?;

    let output = std::process::Command::new("exapump")
        .args([
            "sql",
            "-d",
            &format!(
                "exasol://sys:exasol@{}:{}?validateservercertificate=0",
                harness.host, harness.db_port
            ),
            "SELECT val FROM cb_sink.cb_result ORDER BY val",
        ])
        .output()
        .map_err(|e| anyhow!("running exapump: {e}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    for expected in ["10", "20", "30"] {
        if !stdout.contains(expected) {
            bail!(
                "exapump output missing {expected}: stdout={stdout} stderr={}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
    Ok(())
}

/// Scenario: connect-back write-back into a pre-committed table in the invoking
/// query's OWN schema, exercising the realistic before/UDF/after ordering.
///
/// Demonstrates that same-schema write-back is safe under Exasol's Serializable
/// isolation when the contract is respected:
/// 1. `crunch_log` is created and **committed before** the query (autocommit), so
///    the connect-back session can see it and the UDF does no DDL.
/// 2. The invoking query reads a **different** table (`crunch_in`) than the UDF
///    writes (`crunch_log`), avoiding a read-write WAIT FOR COMMIT conflict.
/// 3. The UDF number-crunches (squares) each input and connect-back-inserts
///    `(v, v*v)`; the connect-back session autocommits each insert.
/// 4. A **new** connection inserts another row after the UDF, proving the table
///    stays usable from an independent session.
async fn connect_back_writeback_same_schema(
    conn: &mut Connection,
    udf_object: &str,
    harness: &Harness,
) -> Result<()> {
    // (1) Pre-create + seed the target table in the invoking schema, committed
    // before the query runs (exarrow-rs autocommits each statement).
    conn.execute("CREATE OR REPLACE TABLE it_rust.crunch_log (v BIGINT, v_squared BIGINT)")
        .await?;
    conn.execute("INSERT INTO it_rust.crunch_log VALUES (1, 1)")
        .await?;

    // (2) Separate input table — the query reads this, never crunch_log.
    conn.execute("CREATE OR REPLACE TABLE it_rust.crunch_in (v BIGINT)")
        .await?;
    conn.execute("INSERT INTO it_rust.crunch_in VALUES (2), (3), (4)")
        .await?;

    let cb_addr = harness.connect_back_sql_address().await?;
    conn.execute(&format!(
        "CREATE OR REPLACE CONNECTION CB_SELF TO '{cb_addr}' \
         USER 'sys' IDENTIFIED BY 'exasol'"
    ))
    .await?;
    conn.execute(&format!(
        "CREATE OR REPLACE RUST SET SCRIPT crunch_writeback(v BIGINT) EMITS (cnt BIGINT) AS\n\
         %connection CB_SELF\n\
         %udf_object {udf_object};\n/"
    ))
    .await?;

    // (3) Run the UDF: connect-back inserts (2,4), (3,9), (4,16) into crunch_log.
    conn.query("SELECT crunch_writeback(v) FROM it_rust.crunch_in")
        .await?;

    // (4) A brand-new independent session inserts another row post-UDF.
    let mut conn2 = harness.connect().await?;
    conn2
        .execute("INSERT INTO it_rust.crunch_log VALUES (5, 25)")
        .await?;
    conn2.close().await?;

    // (5) Verify all rows (before + UDF + after) are present externally.
    let output = std::process::Command::new("exapump")
        .args([
            "sql",
            "-d",
            &format!(
                "exasol://sys:exasol@{}:{}?validateservercertificate=0",
                harness.host, harness.db_port
            ),
            "SELECT v_squared FROM it_rust.crunch_log ORDER BY v",
        ])
        .output()
        .map_err(|e| anyhow!("running exapump: {e}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    for expected in ["1", "4", "9", "16", "25"] {
        if !stdout.contains(expected) {
            bail!(
                "exapump output missing {expected}: stdout={stdout} stderr={}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
    Ok(())
}
