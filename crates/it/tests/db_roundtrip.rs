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

use anyhow::{Result, anyhow, bail};
use exarrow_rs::adbc::Connection;
use it::{Harness, query_single_string, read_udf_artifact, register_slc};

const SCALAR_LIB: &str = "libscalar_double.so";
const SET_LIB: &str = "libset_filter.so";
const JSON_LIB: &str = "libjson_parse.so";
const CB_QUERY_LIB: &str = "libconnect_back_query.so";
const CB_SCALAR_LIB: &str = "libconnect_back_scalar.so";
const CB_CLUSTER_IP_LIB: &str = "libconnect_back_cluster_ip.so";
const SC_LIB: &str = "libsingle_call_fixture.so";
const CB_INSERT_LIB: &str = "libconnect_back_insert.so";
const CB_CRUNCH_LIB: &str = "libconnect_back_crunch.so";
const RESOLV_LIB: &str = "libresolv_udf.so";
const EMIT_BULK_LIB: &str = "libemit_bulk.so";
const EMIT_ARROW_LIB: &str = "libemit_arrow_batch.so";
const CB_STREAM_LIB: &str = "libconnect_back_stream.so";
const TS_ADD_LIB: &str = "libtimestamp_add_second.so";
const TS_NOW_LIB: &str = "libtimestamp_now.so";
const TS_PASS_LIB: &str = "libtimestamp_passthrough.so";
const ANNOTATED_FIXTURE_LIB: &str = "libannotated_fixture.so";
const HANDSHAKE_LIB: &str = "libhandshake_meta.so";

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
            eprintln!(
                "[it] scenario python3_connect_back SKIPPED: could not open diagnostic connection: {e:#}"
            );
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
    let cb_scalar_path = harness
        .upload_udf(CB_SCALAR_LIB, read_udf_artifact(CB_SCALAR_LIB)?)
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
    let resolv_path = harness
        .upload_udf(RESOLV_LIB, read_udf_artifact(RESOLV_LIB)?)
        .await?;
    let emit_bulk_path = harness
        .upload_udf(EMIT_BULK_LIB, read_udf_artifact(EMIT_BULK_LIB)?)
        .await?;
    let emit_arrow_path = harness
        .upload_udf(EMIT_ARROW_LIB, read_udf_artifact(EMIT_ARROW_LIB)?)
        .await?;
    let cb_stream_path = harness
        .upload_udf(CB_STREAM_LIB, read_udf_artifact(CB_STREAM_LIB)?)
        .await?;
    let ts_add_path = harness
        .upload_udf(TS_ADD_LIB, read_udf_artifact(TS_ADD_LIB)?)
        .await?;
    let ts_now_path = harness
        .upload_udf(TS_NOW_LIB, read_udf_artifact(TS_NOW_LIB)?)
        .await?;
    let ts_pass_path = harness
        .upload_udf(TS_PASS_LIB, read_udf_artifact(TS_PASS_LIB)?)
        .await?;
    let annotated_fixture_path = harness
        .upload_udf(
            ANNOTATED_FIXTURE_LIB,
            read_udf_artifact(ANNOTATED_FIXTURE_LIB)?,
        )
        .await?;
    let handshake_path = harness
        .upload_udf(HANDSHAKE_LIB, read_udf_artifact(HANDSHAKE_LIB)?)
        .await?;

    scalar_double_returns_42(&mut conn, &scalar_path).await?;
    eprintln!("[it] scenario scalar_double ok");
    handshake_metadata_udf_emits_session_and_node(&mut conn, &handshake_path).await?;
    eprintln!("[it] scenario handshake_metadata ok");
    annotated_fixture_two_entries_from_one_so(&mut conn, &annotated_fixture_path).await?;
    eprintln!("[it] scenario annotated_fixture_two_entries ok");
    set_filter_emits_positive_only(&mut conn, &set_path).await?;
    eprintln!("[it] scenario set_filter ok");
    json_parse_extracts_name(&mut conn, &json_path).await?;
    eprintln!("[it] scenario json_parse ok");
    udf_error_surfaces_prefix(&mut conn).await?;
    eprintln!("[it] scenario udf_error ok");
    udf_error_message_reaches_db(&mut conn).await?;
    eprintln!("[it] scenario udf_error_message ok");

    emit_bulk_flushes_multiple_batches(&mut conn, &emit_bulk_path).await?;
    eprintln!("[it] scenario emit_bulk ok");

    emit_arrow_batch_roundtrips(&mut conn, &emit_arrow_path).await?;
    eprintln!("[it] scenario emit_arrow_batch ok");

    // Single-call scenarios.
    single_call_default_output_columns_roundtrip(&mut conn, &sc_path).await?;
    eprintln!("[it] scenario single_call_default_output_columns ok");
    single_call_unimplemented_returns_undefined(&mut conn, &sc_path).await?;
    eprintln!("[it] scenario single_call_unimplemented ok");
    single_call_adapter_surfaces_live_handshake_metadata(&mut conn, &sc_path).await?;
    eprintln!("[it] scenario single_call_adapter_handshake_metadata ok");

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

    if let Err(e) =
        connect_back_scalar_queries_and_returns(&mut conn, &cb_scalar_path, &harness).await
    {
        let logs = harness.dump_udf_logs().await;
        eprintln!("[it] UDF logs after connect_back_scalar failure:\n{logs}");
        return Err(e);
    }
    eprintln!("[it] scenario connect_back_scalar ok");

    if let Err(e) = connect_back_writeback_same_schema(&mut conn, &cb_crunch_path, &harness).await {
        let logs = harness.dump_udf_logs().await;
        eprintln!("[it] UDF logs after connect_back_writeback failure:\n{logs}");
        return Err(e);
    }
    eprintln!("[it] scenario connect_back_writeback_same_schema ok");

    if let Err(e) = connect_back_stream_reads_all_rows(&mut conn, &cb_stream_path, &harness).await {
        let logs = harness.dump_udf_logs().await;
        eprintln!("[it] UDF logs after connect_back_stream failure:\n{logs}");
        return Err(e);
    }
    eprintln!("[it] scenario connect_back_stream ok");

    resolv_udf_resolves_external_host(&mut conn, &resolv_path).await?;
    eprintln!("[it] scenario resolv_udf_resolves_external_host ok");
    if let Err(e) = resolv_udf_errors_on_unresolvable_host(&mut conn, &resolv_path).await {
        let logs = harness.dump_udf_logs().await;
        eprintln!("[it] UDF logs after resolv_udf_errors_on_unresolvable_host failure:\n{logs}");
        return Err(e);
    }
    eprintln!("[it] scenario resolv_udf_errors_on_unresolvable_host ok");

    timestamp_arithmetic_roundtrips(&mut conn, &ts_add_path).await?;
    eprintln!("[it] scenario timestamp_arithmetic_roundtrips ok");
    if let Err(e) = udf_local_time_matches_session_tz(&mut conn, &ts_now_path).await {
        let logs = harness.dump_udf_logs().await;
        eprintln!("[it] UDF logs after udf_local_time_matches_session_tz failure:\n{logs}");
        return Err(e);
    }
    eprintln!("[it] scenario udf_local_time_matches_session_tz ok");
    timestamp_precision_matrix_roundtrips(&mut conn, &ts_pass_path).await?;
    eprintln!("[it] scenario timestamp_precision_matrix_roundtrips ok");

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
        "CREATE OR REPLACE RUST SCALAR SCRIPT scalar_double(x BIGINT) RETURNS BIGINT AS\n\
         %udf_object {udf_object};\n/"
    ))
    .await?;

    let got = query_single_string(conn, "SELECT TO_CHAR(scalar_double(21))").await?;
    if got.as_deref() != Some("42") {
        bail!("scalar_double(21) returned {got:?}, expected 42");
    }
    Ok(())
}

/// Scenario: multiple UDFs from ONE `.so`. `annotated-fixture` exports two named
/// entry points (`annotated`, `annotated_double`); we upload it once and create a
/// script per entry, both referencing the same artifact, then assert each resolves
/// to its own entry and runs. This is the live proof of the headline 0.14.0
/// feature: one `.so`, many UDFs, addressed by SQL script name.
///
/// The fixture annotates `input(x: i64), emits(y: i64)`. Two constraints follow:
/// (1) load-time schema validation requires the column ExaType to be exactly
/// `Int64`, so the columns are `DECIMAL(18,0)` (Exasol delivers `DECIMAL(p,0)`
/// fitting i64 as PB_INT64), not `BIGINT` (which arrives as PB_NUMERIC); and
/// (2) the validation matches column NAMES case-sensitively, but Exasol
/// upper-cases unquoted identifiers — so the column names are quoted (`"x"`,
/// `"y"`) to preserve the lower-case names the annotation declares.
async fn annotated_fixture_two_entries_from_one_so(
    conn: &mut Connection,
    udf_object: &str,
) -> Result<()> {
    conn.execute(&format!(
        "CREATE OR REPLACE RUST SET SCRIPT it_rust.annotated(\"x\" DECIMAL(18,0)) \
         EMITS (\"y\" DECIMAL(18,0)) AS\n\
         %udf_object {udf_object};\n/"
    ))
    .await?;
    conn.execute(&format!(
        "CREATE OR REPLACE RUST SET SCRIPT it_rust.annotated_double(\"x\" DECIMAL(18,0)) \
         EMITS (\"y\" DECIMAL(18,0)) AS\n\
         %udf_object {udf_object};\n/"
    ))
    .await?;

    let identity = query_single_string(
        conn,
        "SELECT TO_CHAR(y) FROM (SELECT annotated(CAST(21 AS DECIMAL(18,0))) AS y FROM DUAL)",
    )
    .await?;
    if identity.as_deref() != Some("21") {
        bail!("annotated(21) returned {identity:?}, expected 21 (identity entry point)");
    }

    let doubled = query_single_string(
        conn,
        "SELECT TO_CHAR(y) FROM (SELECT annotated_double(CAST(21 AS DECIMAL(18,0))) AS y FROM DUAL)",
    )
    .await?;
    if doubled.as_deref() != Some("42") {
        bail!(
            "annotated_double(21) returned {doubled:?}, expected 42 — the second \
             named entry point in the same .so did not resolve correctly"
        );
    }
    Ok(())
}

/// Scenario: live handshake metadata reaches UDF code through the `UdfContext`
/// accessors. The `handshake_meta` SCALAR fixture emits one pipe-delimited
/// string built from `ctx.session_id()`, `ctx.node_id()`, `ctx.node_count()`,
/// and `ctx.script_name()`. We assert the values are LIVE, not the neutral
/// defaults the accessors return on a context that does not override them:
///
///  - `session_id` is non-deterministic but a real session always has a
///    non-zero ID, so the neutral `0` default would fail this gate.
///  - `node_id` is 0-based, so on single-node Docker it is legitimately `0`;
///    we assert only that it parses as a valid u32 (present), not non-zero.
///  - `node_count` is `>= 1` for any live cluster, so the neutral `0` default
///    would fail — this is the node-metadata liveness gate.
///  - `script_name` must match the registered script name. Exasol upper-cases
///    unquoted identifiers, so we compare case-insensitively and require the
///    registered `handshake_meta` name to appear (the neutral default is the
///    empty string).
async fn handshake_metadata_udf_emits_session_and_node(
    conn: &mut Connection,
    udf_object: &str,
) -> Result<()> {
    conn.execute(&format!(
        "CREATE OR REPLACE RUST SCALAR SCRIPT handshake_meta() RETURNS VARCHAR(2000) AS\n\
         %udf_object {udf_object};\n/"
    ))
    .await?;

    let got = query_single_string(conn, "SELECT TO_CHAR(handshake_meta())").await?;
    let summary = got.ok_or_else(|| anyhow!("handshake_meta returned NULL"))?;

    let parts: Vec<&str> = summary.split('|').collect();
    if parts.len() != 4 {
        bail!("handshake_meta emitted {summary:?}, expected 4 pipe-delimited fields");
    }

    let session_id: u64 = parts[0]
        .parse()
        .map_err(|e| anyhow!("session_id {:?} not a u64: {e}", parts[0]))?;
    if session_id == 0 {
        bail!(
            "handshake_meta session_id is 0 (the neutral default) — live handshake \
             metadata did not reach the UDF: {summary:?}"
        );
    }

    // node_id is 0-based; single-node Docker is node 0, so liveness is proven by
    // node_count, not node_id. Assert node_id is present/valid only.
    parts[1]
        .parse::<u32>()
        .map_err(|e| anyhow!("node_id {:?} not a u32: {e}", parts[1]))?;

    let node_count: u32 = parts[2]
        .parse()
        .map_err(|e| anyhow!("node_count {:?} not a u32: {e}", parts[2]))?;
    if node_count == 0 {
        bail!(
            "handshake_meta node_count is 0 (the neutral default) — live node \
             metadata did not reach the UDF: {summary:?}"
        );
    }

    let script_name = parts[3];
    if !script_name.to_ascii_uppercase().contains("HANDSHAKE_META") {
        bail!(
            "handshake_meta script_name {script_name:?} does not match the \
             registered script name HANDSHAKE_META: {summary:?}"
        );
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
        "CREATE OR REPLACE RUST SET SCRIPT set_filter(x BIGINT) EMITS (y BIGINT) AS\n\
         %udf_object {udf_object};\n/"
    ))
    .await?;

    let count = query_single_string(
        conn,
        "SELECT TO_CHAR(COUNT(*)) FROM (SELECT set_filter(x) FROM nums)",
    )
    .await?
    .ok_or_else(|| anyhow!("count query returned NULL"))?;
    if count != expected_positive.to_string() {
        bail!("filter_positive emitted {count} rows, expected {expected_positive}");
    }

    let min_emitted = query_single_string(
        conn,
        "SELECT TO_CHAR(MIN(y)) FROM (SELECT set_filter(x) AS y FROM nums)",
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
        "CREATE OR REPLACE RUST SCALAR SCRIPT json_parse(doc VARCHAR(2000)) RETURNS VARCHAR(2000) AS\n\
         %udf_object {udf_object};\n/"
    ))
    .await?;

    let got = query_single_string(conn, "SELECT json_parse('{\"name\":\"exa\"}')").await?;
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
    let result = conn.query("SELECT json_parse('not valid json')").await;
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

/// Scenario 8.8: the UDF-supplied error text propagates all the way to the SQL
/// error message, not just the `F-UDF-CL-RUST-` prefix. `json_field` is already
/// registered by `json_parse_extracts_name`; calling it with invalid JSON should
/// produce a SQL error that contains "JSON parse error" — the distinctive text
/// returned by `UdfError::User(format!("JSON parse error: ..."))` inside the UDF.
/// This proves the runtime threads the error string from the ABI `error_out`
/// parameter through the protobuf error-close path to the client.
async fn udf_error_message_reaches_db(conn: &mut Connection) -> Result<()> {
    let result = conn.query("SELECT json_parse('not valid json')").await;
    match result {
        Ok(_) => bail!("expected UDF error, query succeeded"),
        Err(e) => {
            let msg = e.to_string();
            if !msg.contains("JSON parse error") {
                bail!("error did not contain 'JSON parse error': {msg}");
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

/// Scenario: connect-back from a SCALAR script — the UDF issues `SELECT 42` via
/// connect-back and RETURNS it. Proves connect-back is not SET-only; a SCALAR
/// `RETURNS` script connects back without the historical SIGABRT.
async fn connect_back_scalar_queries_and_returns(
    conn: &mut Connection,
    udf_object: &str,
    harness: &Harness,
) -> Result<()> {
    let cb_addr = harness.connect_back_sql_address().await?;
    eprintln!("[it] connect_back_scalar: CB_SELF address = {cb_addr}");
    conn.execute(&format!(
        "CREATE OR REPLACE CONNECTION CB_SELF TO '{cb_addr}' \
         USER 'sys' IDENTIFIED BY 'exasol'"
    ))
    .await?;
    conn.execute(&format!(
        "CREATE OR REPLACE RUST SCALAR SCRIPT connect_back_scalar() RETURNS BIGINT AS\n\
         %udf_object {udf_object};\n/"
    ))
    .await?;
    let got = query_single_string(conn, "SELECT TO_CHAR(connect_back_scalar())").await?;
    if got.as_deref() != Some("42") {
        bail!("connect_back_scalar returned {got:?}, expected 42");
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
        "CREATE OR REPLACE RUST SCALAR SCRIPT single_call_udf() RETURNS VARCHAR(2000) AS\n\
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

/// Extract the digits following `<key>=` in the adapter error text and parse
/// them as a `u64`. The text is embedded in a wrapped SQL error, so we locate
/// the `key=` marker and consume the contiguous run of ASCII digits after it.
fn parse_meta_u64(haystack: &str, key: &str) -> Result<u64> {
    let marker = format!("{key}=");
    let start = haystack
        .find(&marker)
        .ok_or_else(|| anyhow!("{key} not present in adapter error text: {haystack:?}"))?
        + marker.len();
    let digits: String = haystack[start..]
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    if digits.is_empty() {
        bail!("{key} has no numeric value in adapter error text: {haystack:?}");
    }
    digits
        .parse::<u64>()
        .map_err(|e| anyhow!("{key} {digits:?} not a u64: {e}"))
}

/// Scenario: a `createVirtualSchema` adapter single-call
/// (`SC_FN_VIRTUAL_SCHEMA_ADAPTER_CALL`) sees live handshake metadata.
///
/// The `single-call-fixture` adapter shim reads the live `UdfContext` handshake
/// accessors and returns rc=1 with an error string of the form
/// `HANDSHAKE_META node_count=<n> node_id=<n> session_id=<n> script_name=<s>`,
/// so the `CREATE VIRTUAL SCHEMA` is expected to FAIL with that metadata in the
/// surfaced error text. A non-zero `node_count` proves the fix for issue #41 —
/// the single-call accessors previously returned the neutral default `0` for a
/// cluster of any size. Mirrors `handshake_metadata_udf_emits_session_and_node`:
/// same neutral-default gate, same parse-and-check style.
async fn single_call_adapter_surfaces_live_handshake_metadata(
    conn: &mut Connection,
    udf_object: &str,
) -> Result<()> {
    // The fixture exports a single entry point, `__exa_udf_entry_SINGLE_CALL_UDF`,
    // and the loader resolves the entry point from the (uppercased) script name.
    // So the adapter script MUST be named `single_call_udf`. An earlier scenario
    // registered a SCALAR script of that name; drop it first so the ADAPTER
    // script can take the name cleanly.
    conn.execute("DROP SCRIPT IF EXISTS single_call_udf").await?;
    conn.execute(&format!(
        "CREATE OR REPLACE RUST ADAPTER SCRIPT single_call_udf AS\n\
         %udf_object {udf_object};\n/"
    ))
    .await?;

    let result = conn
        .execute("CREATE VIRTUAL SCHEMA vs_handshake_meta USING it_rust.single_call_udf")
        .await;
    let summary = match result {
        Ok(_) => {
            // The shim always returns rc=1, so a successful create means the live
            // metadata never reached the adapter hook. Best-effort cleanup.
            let _ = conn
                .execute("DROP VIRTUAL SCHEMA IF EXISTS vs_handshake_meta CASCADE")
                .await;
            bail!(
                "CREATE VIRTUAL SCHEMA unexpectedly succeeded — the adapter shim \
                 should have failed with live handshake metadata in the error text"
            );
        }
        Err(e) => e.to_string(),
    };

    if !summary.contains("HANDSHAKE_META") {
        bail!("adapter error did not carry the HANDSHAKE_META marker: {summary:?}");
    }

    // Core assertion: node_count != 0 proves live handshake metadata reached the
    // virtual-schema adapter call (the neutral default is 0).
    let node_count = parse_meta_u64(&summary, "node_count")?;
    if node_count == 0 {
        bail!(
            "adapter node_count is 0 (the neutral default) — live handshake \
             metadata did not reach the virtual-schema adapter call: {summary:?}"
        );
    }

    // node_id is 0-based (single-node Docker is node 0), so only assert it is
    // present and parseable; session_id likewise must be present and parseable.
    parse_meta_u64(&summary, "node_id")?;
    parse_meta_u64(&summary, "session_id")?;

    if !summary.to_ascii_uppercase().contains("SINGLE_CALL_UDF") {
        bail!(
            "adapter error text does not contain the registered script name \
             SINGLE_CALL_UDF: {summary:?}"
        );
    }
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

/// Scenario: resolv_udf resolves an external hostname to a valid IP address.
async fn resolv_udf_resolves_external_host(conn: &mut Connection, udf_object: &str) -> Result<()> {
    conn.execute(&format!(
        "CREATE OR REPLACE RUST SCALAR SCRIPT resolv_udf(host VARCHAR(2000)) RETURNS VARCHAR(2000) AS\n\
         %udf_object {udf_object};\n/"
    ))
    .await?;
    let got = query_single_string(conn, "SELECT resolv_udf('www.exasol.com')").await?;
    let ip = got.ok_or_else(|| anyhow!("resolv_udf returned NULL"))?;
    ip.parse::<std::net::IpAddr>()
        .map_err(|_| anyhow!("resolv_udf('www.exasol.com') returned non-IP: {ip:?}"))?;
    Ok(())
}

/// Scenario: resolv_udf surfaces an error for an unresolvable hostname.
async fn resolv_udf_errors_on_unresolvable_host(
    conn: &mut Connection,
    _udf_object: &str,
) -> Result<()> {
    let result = conn
        .query("SELECT resolv_udf('this-host-definitely-does-not-exist.invalid')")
        .await;
    match result {
        Ok(_) => bail!("expected DNS error, query succeeded"),
        Err(e) => {
            let msg = e.to_string();
            if !msg.contains("F-UDF-CL-RUST-") {
                bail!("error did not contain F-UDF-CL-RUST- prefix: {msg}");
            }
            Ok(())
        }
    }
}

/// Verify that the 4,000,000-byte mid-run flush works end-to-end:
/// emitting 50,000 rows × ~98 bytes = ~4.9 MB forces at least one mid-run
/// MT_EMIT flush plus a tail flush; all rows must arrive intact.
async fn emit_bulk_flushes_multiple_batches(conn: &mut Connection, lib_path: &str) -> Result<()> {
    const N: i64 = 50_000;
    conn.execute(&format!(
        "CREATE OR REPLACE RUST SET SCRIPT it_rust.emit_bulk(n BIGINT) \
         EMITS (val VARCHAR(200)) AS\n\
         %udf_object {lib_path};\n/"
    ))
    .await?;
    let result = query_single_string(
        conn,
        "SELECT TO_CHAR(COUNT(*)) \
         FROM (SELECT emit_bulk(50000) FROM DUAL)",
    )
    .await?
    .ok_or_else(|| anyhow!("emit_bulk count query returned NULL"))?;
    let count: i64 = result.parse().map_err(|e| anyhow!("parse count: {e}"))?;
    if count != N {
        bail!("emit_bulk: expected {N} rows, got {count}");
    }
    Ok(())
}

/// Scenario: `emit_arrow_batch` — SET UDF that builds a 3-row Arrow RecordBatch
/// and calls `ctx.emit_batch(&batch)` once. Asserts count == 3 and that the
/// exact (id, label) pairs (1:a, 2:b, 3:c) arrive in order.
async fn emit_arrow_batch_roundtrips(conn: &mut Connection, lib_path: &str) -> Result<()> {
    conn.execute(&format!(
        "CREATE OR REPLACE RUST SET SCRIPT it_rust.emit_arrow_batch(dummy BOOLEAN) \
         EMITS (id BIGINT, label VARCHAR(1)) AS\n\
         %udf_object {lib_path};\n/"
    ))
    .await?;

    let count = query_single_string(
        conn,
        "SELECT TO_CHAR(COUNT(*)) FROM (SELECT emit_arrow_batch(TRUE) FROM DUAL)",
    )
    .await?
    .ok_or_else(|| anyhow!("emit_arrow_batch count query returned NULL"))?;
    if count != "3" {
        bail!("emit_arrow_batch: expected 3 rows, got {count}");
    }

    let aggregated = query_single_string(
        conn,
        "SELECT GROUP_CONCAT(TO_CHAR(id) || ':' || label ORDER BY id) \
         FROM (SELECT emit_arrow_batch(TRUE) FROM DUAL)",
    )
    .await?
    .ok_or_else(|| anyhow!("emit_arrow_batch aggregation query returned NULL"))?;
    if aggregated != "1:a,2:b,3:c" {
        bail!("emit_arrow_batch: expected '1:a,2:b,3:c', got {aggregated:?}");
    }
    Ok(())
}

/// Verify that `query_for_each` streams all rows from a seeded table via
/// connect-back: the UDF emits the total row count and we assert it equals M.
async fn connect_back_stream_reads_all_rows(
    conn: &mut Connection,
    lib_path: &str,
    harness: &Harness,
) -> Result<()> {
    const M: i64 = 100;
    let cb_addr = harness.connect_back_sql_address().await?;
    // CB_SELF may already exist from earlier scenarios; CREATE OR REPLACE is safe.
    conn.execute(&format!(
        "CREATE OR REPLACE CONNECTION CB_SELF TO '{cb_addr}' \
         USER 'sys' IDENTIFIED BY 'exasol'"
    ))
    .await?;

    // Seed the table with M rows; recreate for idempotency.
    conn.execute("DROP TABLE IF EXISTS it_rust.cb_stream_seed")
        .await?;
    conn.execute("CREATE TABLE it_rust.cb_stream_seed (id INTEGER)")
        .await?;
    let values: String = (1..=M)
        .map(|i| format!("({i})"))
        .collect::<Vec<_>>()
        .join(",");
    conn.execute(&format!(
        "INSERT INTO it_rust.cb_stream_seed VALUES {values}"
    ))
    .await?;

    conn.execute(&format!(
        "CREATE OR REPLACE RUST SET SCRIPT it_rust.connect_back_stream(dummy BOOLEAN) \
         EMITS (row_count BIGINT) AS\n\
         %udf_object {lib_path};\n/"
    ))
    .await?;

    let result = query_single_string(
        conn,
        "SELECT TO_CHAR(row_count) \
         FROM (SELECT connect_back_stream(TRUE) FROM DUAL)",
    )
    .await?
    .ok_or_else(|| anyhow!("connect_back_stream returned NULL"))?;
    let count: i64 = result.parse().map_err(|e| anyhow!("parse count: {e}"))?;
    if count != M {
        bail!("connect_back_stream: expected {M} rows, got {count}");
    }
    Ok(())
}

/// Scenario: timestamp arithmetic round-trips through a SCALAR UDF.
///
/// `ts_add_second(t TIMESTAMP) RETURNS TIMESTAMP` reads a `Value::Timestamp`,
/// adds one second, and emits it. We assert the result equals the input plus
/// exactly one second AND that the sub-second `.250` component survived the
/// decode/emit round-trip (a zeroing/truncation bug would make the equality
/// fail). The comparison is done SQL-side via `CASE WHEN ... = TIMESTAMP '...'`
/// so it is robust to `TO_CHAR` fractional-second formatting quirks.
async fn timestamp_arithmetic_roundtrips(conn: &mut Connection, udf_object: &str) -> Result<()> {
    conn.execute(&format!(
        "CREATE OR REPLACE RUST SCALAR SCRIPT timestamp_add_second(t TIMESTAMP) RETURNS TIMESTAMP AS\n\
         %udf_object {udf_object};\n/"
    ))
    .await?;

    // Plain TIMESTAMP is precision-3, so `.250` fits without engine truncation.
    let got = query_single_string(
        conn,
        "SELECT CASE \
           WHEN timestamp_add_second(TIMESTAMP '2026-06-14 09:30:15.250000') \
              = TIMESTAMP '2026-06-14 09:30:16.250000' \
           THEN 'eq' ELSE 'ne' END",
    )
    .await?
    .ok_or_else(|| anyhow!("ts_add_second comparison returned NULL"))?;
    if got != "eq" {
        bail!(
            "ts_add_second(2026-06-14 09:30:15.250000) did not equal \
             2026-06-14 09:30:16.250000 (got {got:?}); the +1s arithmetic or the \
             sub-second .250 component did not survive the round-trip"
        );
    }
    Ok(())
}

/// Scenario: the UDF's local wall-clock agrees with the session timezone and is
/// NOT UTC — the regression gate for the `tzdata` packaging fix.
///
/// `udf_now() RETURNS TIMESTAMP` emits `chrono::Local::now().naive_local()`,
/// i.e. the container's local wall-clock resolved from `TZ` + the bundled
/// `/usr/share/zoneinfo`. With `tzdata` present the emitted naive value carries
/// the Berlin wall-clock; WITHOUT `tzdata` `chrono::Local` silently falls back
/// to UTC and the offset assertion below reports ~0 instead of the Berlin
/// offset — so this scenario FAILS on an Alpine image built without `tzdata`
/// and PASSES with it.
///
/// Two SQL-side properties are asserted (no fragile client-side timestamp
/// parsing — `AT TIME ZONE` is not supported by this engine, so the UTC instant
/// is obtained via `POSIX_TIME()`):
///
///  (a) Bounded skew: `ABS(SECONDS_BETWEEN(udf_now(), CURRENT_TIMESTAMP))` is
///      within a few seconds, covering UDF execution latency. Both values are
///      the Berlin wall-clock, so they must agree closely.
///  (b) Non-UTC Berlin offset: the UDF's naive value, interpreted as
///      seconds-since-epoch, MUST differ from the true UTC epoch
///      (`POSIX_TIME()`) by the Berlin UTC offset (3600s for CET or 7200s for
///      CEST). `SECONDS_BETWEEN(ts, TIMESTAMP '1970-01-01 00:00:00')` treats
///      `ts` as a naive value, so subtracting `POSIX_TIME()` yields the offset
///      baked into the emitted wall-clock. A UTC fallback yields ~0 and fails.
async fn udf_local_time_matches_session_tz(conn: &mut Connection, udf_object: &str) -> Result<()> {
    conn.execute(&format!(
        "CREATE OR REPLACE RUST SCALAR SCRIPT timestamp_now() RETURNS TIMESTAMP AS\n\
         %udf_object {udf_object};\n/"
    ))
    .await?;

    // Named IANA zone; verified accepted form against the live DB
    // (SESSIONTIMEZONE reports 'EUROPE/BERLIN').
    conn.execute("ALTER SESSION SET TIME_ZONE='Europe/Berlin'")
        .await?;

    // (a) Bounded skew between the UDF wall-clock and the DB wall-clock.
    let skew = query_single_string(
        conn,
        "SELECT TO_CHAR(ABS(SECONDS_BETWEEN(timestamp_now(), CURRENT_TIMESTAMP)))",
    )
    .await?
    .ok_or_else(|| anyhow!("udf_now skew query returned NULL"))?;
    let skew_secs: f64 = skew
        .parse()
        .map_err(|e| anyhow!("parsing udf_now skew {skew:?}: {e}"))?;
    // Generous tolerance for container/execution latency.
    if skew_secs > 30.0 {
        bail!(
            "udf_now() disagrees with CURRENT_TIMESTAMP by {skew_secs}s \
             (> 30s tolerance); the UDF wall-clock is not tracking the session clock"
        );
    }

    // (b) The emitted wall-clock carries the Berlin offset, not UTC.
    // offset = (udf_now interpreted as naive epoch seconds) - (true UTC epoch).
    let offset = query_single_string(
        conn,
        "SELECT TO_CHAR(\
            SECONDS_BETWEEN(timestamp_now(), TIMESTAMP '1970-01-01 00:00:00') - POSIX_TIME())",
    )
    .await?
    .ok_or_else(|| anyhow!("udf_now offset query returned NULL"))?;
    let offset_secs: f64 = offset
        .parse()
        .map_err(|e| anyhow!("parsing udf_now offset {offset:?}: {e}"))?;
    // Accept CET (3600s) or CEST (7200s); the +-30s slack absorbs the latency
    // between the two function evaluations. A UTC fallback (no tzdata) gives ~0
    // and fails this assertion — exactly the regression we are gating.
    let is_cet = (offset_secs - 3600.0).abs() <= 30.0;
    let is_cest = (offset_secs - 7200.0).abs() <= 30.0;
    if !(is_cet || is_cest) {
        bail!(
            "udf_now() offset from UTC is {offset_secs}s, expected the Berlin \
             offset (~3600s CET or ~7200s CEST). ~0 means the UDF reported UTC \
             — the tzdata packaging regression"
        );
    }
    Ok(())
}

/// Scenario: TIMESTAMP fractional precision round-trips through the engine's
/// receipt-side truncation for the 0/3/6/9 matrix.
///
/// `timestamp-passthrough` reads a `Value::Timestamp` and re-emits it unchanged.
/// For each precision `p` we register `ts_pass_p(t TIMESTAMP(p)) RETURNS
/// TIMESTAMP(p)` over the same `.so`, feed a literal carrying exactly `p`
/// fractional digits (the base `123456789` truncated to `p`), and assert the
/// returned value equals what survives a UDF round-trip at that precision.
///
/// The DB caps the fractional precision of every UDF *input* column at
/// microseconds — `SWIGTableData::getTimestamp` formats `...FF6` for all script
/// languages (verified against the engine source and empirically across Rust,
/// Python, and Java; see decision-log [2]). The emit side carries all 9 digits
/// (`%.9f`) and the engine truncates them to the column's declared precision on
/// receipt. So the realistic round-trip ceiling through a UDF is microseconds:
/// the expected stored value is the literal truncated to `min(p, 6)` fractional
/// digits, then widened back to `TIMESTAMP(p)`.
///
/// For `p ∈ {0,3,6}` that is exactly `CAST(LIT AS TIMESTAMP(p))` (lossless,
/// nothing beyond microseconds to lose). `p = 9` is the input-cap case: the DB
/// delivers `.123456000` to the UDF, so the round-trip yields `.123456000`, not
/// the stored literal `.123456789`. `p = 0` must NOT gain a spurious `.000000` —
/// the engine truncates the emitted fraction away, so the equality still holds.
async fn timestamp_precision_matrix_roundtrips(
    conn: &mut Connection,
    udf_object: &str,
) -> Result<()> {
    // base fractional digits: 123456789; truncate to `p` digits per precision.
    for p in [0u32, 3, 6, 9] {
        let frac = match p {
            0 => String::new(),
            _ => format!(".{}", &"123456789"[..p as usize]),
        };
        let literal = format!("2026-06-14 09:30:15{frac}");

        // The DB delivers UDF input columns at microsecond precision (FF6), so the
        // value the UDF can return is the literal truncated to min(p,6) digits and
        // widened back to TIMESTAMP(p). For p<=6 this equals CAST(LIT AS TIMESTAMP(p)).
        let input_cap = p.min(6);

        // One UDF entry point (`timestamp_passthrough`) backs every precision; the
        // script name must equal that entry name, so the single name is reused and
        // CREATE OR REPLACE'd per precision rather than suffixed with `p`.
        let script = "timestamp_passthrough";
        conn.execute(&format!(
            "CREATE OR REPLACE RUST SCALAR SCRIPT {script}(t TIMESTAMP({p})) \
             RETURNS TIMESTAMP({p}) AS\n\
             %udf_object {udf_object};\n/"
        ))
        .await?;

        let got = query_single_string(
            conn,
            &format!(
                "SELECT CASE \
                   WHEN {script}(TIMESTAMP '{literal}') \
                      = CAST(CAST(TIMESTAMP '{literal}' AS TIMESTAMP({input_cap})) \
                             AS TIMESTAMP({p})) \
                   THEN 'eq' ELSE 'ne' END"
            ),
        )
        .await?
        .ok_or_else(|| anyhow!("{script} comparison returned NULL"))?;
        if got != "eq" {
            bail!(
                "{script}(TIMESTAMP '{literal}') did not match the expected \
                 microsecond-capped round-trip at TIMESTAMP({p}) (got {got:?}); \
                 expected the literal truncated to {input_cap} fractional digits \
                 (the DB delivers UDF inputs at FF6/microsecond precision)"
            );
        }
    }
    Ok(())
}
