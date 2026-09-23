//! Fixture for the `cleanup(path)` hook. Entry points with statics serve one
//! test each, since the `.so` stays mapped for the whole test process.

use exasol_udf_macros::exasol_udf;
use exasol_udf_sdk::connect_back::ConnectionObject;
use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::error::UdfError;
use exasol_udf_sdk::value::Value;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicI64, Ordering};

#[exasol_udf(cleanup(succeed))]
pub fn cleanup_ok(ctx: &mut dyn UdfContext) -> Result<Option<Value>, UdfError> {
    ctx.get_value(0)
}

fn succeed(_ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    Ok(())
}

static REPORTED_GROUPS: AtomicI64 = AtomicI64::new(0);
static REPORTED_ROWS: AtomicI64 = AtomicI64::new(0);

#[exasol_udf(cleanup(report_counts))]
pub fn cleanup_reports(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    let mut rows = 0;
    while ctx.next()? {
        rows += 1;
    }
    REPORTED_GROUPS.fetch_add(1, Ordering::Relaxed);
    REPORTED_ROWS.fetch_add(rows, Ordering::Relaxed);
    ctx.emit(vec![Value::Int64(rows)])
}

fn report_counts(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    let groups = REPORTED_GROUPS.load(Ordering::Relaxed);
    if groups == 0 {
        return Ok(());
    }
    Err(UdfError::User(format!(
        "cleanup ran: script={} groups={groups} rows={}",
        ctx.script_name(),
        REPORTED_ROWS.load(Ordering::Relaxed),
    )))
}

#[exasol_udf(input(x: i64), cleanup(fail_cleanup))]
pub fn cleanup_after_run_error(_ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    Err(UdfError::User("run failed on purpose".into()))
}

fn fail_cleanup(_ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    Err(UdfError::User("cleanup failed on purpose".into()))
}

static RESOLVED_CONNECTION: OnceLock<ConnectionObject> = OnceLock::new();

#[exasol_udf(cleanup(read_over_resolved_connection))]
pub fn cleanup_connect_back(ctx: &mut dyn UdfContext) -> Result<Option<Value>, UdfError> {
    if RESOLVED_CONNECTION.get().is_none() {
        let conn = ctx.connection("CB_SELF")?;
        RESOLVED_CONNECTION.get_or_init(|| conn);
    }
    ctx.get_value(0)
}

fn read_over_resolved_connection(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    let Some(conn) = RESOLVED_CONNECTION.get() else {
        return Ok(());
    };
    let connection_refused = ctx.connection("CB_SELF").is_err();
    let rows = ctx.connect_back(conn)?.query("SELECT CAST(42 AS BIGINT)")?;
    Err(UdfError::User(format!(
        "cleanup connect-back read {rows:?} connection_refused={connection_refused}"
    )))
}

#[exasol_udf(export_spec(select_one), cleanup(fail_cleanup))]
pub fn export_cleanup(_ctx: &mut dyn UdfContext) -> Result<Option<i64>, UdfError> {
    Ok(None)
}

fn select_one(_ctx: &mut dyn UdfContext, _json_spec: &str) -> Result<String, UdfError> {
    Ok("SELECT 1".into())
}
