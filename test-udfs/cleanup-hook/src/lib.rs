//! Fixture for the `cleanup(path)` hook: one entry point per cleanup behavior
//! the runtime's mock-database tests and the live scenarios pin.
//!
//! The `.so` stays mapped across the tests of one process, so an entry point
//! that keeps state in statics serves exactly one test.

use exasol_udf_macros::exasol_udf;
use exasol_udf_sdk::connect_back::ConnectionObject;
use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::error::UdfError;
use exasol_udf_sdk::value::Value;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicI64, Ordering};

#[exasol_udf(cleanup(succeed))]
pub fn cleanup_ok(ctx: &mut dyn UdfContext) -> Result<Option<Value>, UdfError> {
    echo_input(ctx)
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

/// Fails with what the process accumulated, so a test reads the counts cleanup
/// observed over every group that process ran.
fn report_counts(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    let groups = REPORTED_GROUPS.load(Ordering::Relaxed);
    if groups == 0 {
        return Ok(());
    }
    let io_rejected =
        ctx.next().is_err() && ctx.get(0).is_err() && ctx.emit(vec![Value::Int64(0)]).is_err();
    Err(UdfError::User(format!(
        "cleanup ran: script={} groups={groups} rows={} io_rejected={io_rejected}",
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

#[exasol_udf(cleanup(look_up_connection))]
pub fn cleanup_connection(_ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    Ok(())
}

fn look_up_connection(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    ctx.connection("CB_SELF").map(drop)
}

static RESOLVED_CONNECTION: OnceLock<ConnectionObject> = OnceLock::new();

#[exasol_udf(cleanup(read_over_resolved_connection))]
pub fn cleanup_connect_back(ctx: &mut dyn UdfContext) -> Result<Option<Value>, UdfError> {
    if RESOLVED_CONNECTION.get().is_none() {
        let conn = ctx.connection("CB_SELF")?;
        RESOLVED_CONNECTION.get_or_init(|| conn);
    }
    echo_input(ctx)
}

/// Fails with the value it read over the connection `run()` resolved, and
/// whether its own CONNECTION lookup was refused for the cleanup phase.
fn read_over_resolved_connection(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    let Some(conn) = RESOLVED_CONNECTION.get() else {
        return Ok(());
    };
    let connection_refused = ctx
        .connection("CB_SELF")
        .is_err_and(|e| e.to_string().contains("during cleanup"));
    let mut session = ctx.connect_back(conn)?;
    let rows = session.query("SELECT CAST(42 AS BIGINT)")?;
    let read = first_integer(&rows)?;
    Err(UdfError::User(format!(
        "cleanup connect-back read {read} connection_refused={connection_refused}"
    )))
}

#[exasol_udf(export_spec(select_one), cleanup(report_single_call_refusal))]
pub fn export_cleanup(_ctx: &mut dyn UdfContext) -> Result<Option<i64>, UdfError> {
    Ok(None)
}

fn select_one(_ctx: &mut dyn UdfContext, _json_spec: &str) -> Result<String, UdfError> {
    Ok("SELECT 1".into())
}

fn report_single_call_refusal(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    let refusal = ctx
        .connection("CB_SELF")
        .err()
        .map(|e| e.to_string())
        .unwrap_or_default();
    Err(UdfError::User(format!(
        "cleanup ran after export_spec: {refusal}"
    )))
}

fn echo_input(ctx: &dyn UdfContext) -> Result<Option<Value>, UdfError> {
    match ctx.get(0)? {
        Value::Null => Ok(None),
        value => Ok(Some(value.clone())),
    }
}

fn first_integer(rows: &[Vec<Value>]) -> Result<i64, UdfError> {
    match rows.first().and_then(|row| row.first()) {
        Some(Value::Int64(n)) => Ok(*n),
        Some(Value::Numeric(d)) if d.scale == 0 => i64::try_from(d.unscaled)
            .map_err(|_| UdfError::Type(format!("connect-back value {d} overflows i64"))),
        other => Err(UdfError::Type(format!(
            "connect-back read returned {other:?}, expected one integer"
        ))),
    }
}
