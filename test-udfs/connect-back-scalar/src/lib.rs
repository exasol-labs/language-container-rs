use exasol_udf_macros::exasol_udf;
use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::error::UdfError;
use exasol_udf_sdk::value::{Decimal, Value};

/// SCALAR UDF that issues a connect-back query (`SELECT CAST(42 AS BIGINT)`) and
/// returns the first cell of the first result row.
///
/// Identical connect-back logic to the `connect-back-query` SET UDF — the only
/// difference is registration (`RUST SCALAR SCRIPT ... RETURNS BIGINT` vs
/// `SET SCRIPT ... EMITS`). A scalar UDF returns its single value the same way
/// (`ctx.emit` of one row), so the runtime dispatch path is shared. This proves
/// connect-back works from a SCALAR script, not just SET/EMITS.
#[exasol_udf]
pub fn connect_back_scalar(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    let c = ctx.connection("CB_SELF")?;
    let mut conn = ctx.connect_back(&c)?;
    let rows = conn.query("SELECT CAST(42 AS BIGINT)")?;
    let cell = rows
        .first()
        .and_then(|r| r.first())
        .ok_or_else(|| UdfError::User("connect_back_scalar: empty result".into()))?;
    let val: i64 = match cell {
        Value::Int64(n) => *n,
        Value::Int32(n) => *n as i64,
        Value::Numeric(d) if d.scale == 0 => i64::try_from(d.unscaled)
            .map_err(|_| UdfError::Type(format!("Numeric value {} overflows i64", d)))?,
        other => return Err(UdfError::Type(format!("unexpected value {other:?}"))),
    };
    // BIGINT scalar output travels as PB_NUMERIC (typed Decimal).
    ctx.emit(&[Value::Numeric(Decimal {
        unscaled: val as i128,
        scale: 0,
    })])
}
