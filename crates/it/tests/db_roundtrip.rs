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

#[tokio::test(flavor = "multi_thread")]
async fn db_roundtrip_all_scenarios() -> Result<()> {
    eprintln!("[it] starting exasol container");
    let harness = Harness::start().await?;
    eprintln!("[it] container up; connecting");
    let mut conn = harness.connect().await?;
    eprintln!("[it] connected");

    sanity_select_one(&mut conn).await?;
    eprintln!("[it] SELECT 1 ok");

    // Register the slim Rust SLC for this session, then make a working schema.
    eprintln!("[it] exporting + uploading SLC to BucketFS");
    let slc = harness.load_slc().await?;
    eprintln!("[it] SLC uploaded; registering SCRIPT_LANGUAGES");
    register_slc(&mut conn, &slc).await?;
    conn.execute("CREATE SCHEMA IF NOT EXISTS it_rust").await?;
    conn.execute("OPEN SCHEMA it_rust").await?;
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

    scalar_double_returns_42(&mut conn, &scalar_path).await?;
    eprintln!("[it] scenario scalar_double ok");
    set_filter_emits_positive_only(&mut conn, &set_path).await?;
    eprintln!("[it] scenario set_filter ok");
    json_parse_extracts_name(&mut conn, &json_path).await?;
    eprintln!("[it] scenario json_parse ok");
    udf_error_surfaces_prefix(&mut conn).await?;
    eprintln!("[it] scenario udf_error ok");

    conn.close().await?;
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
