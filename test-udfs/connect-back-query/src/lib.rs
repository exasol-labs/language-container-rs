use exasol_udf_macros::exasol_udf;
use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::error::UdfError;
use exasol_udf_sdk::value::{Decimal, Value};

/// SET UDF that issues a connect-back query (`SELECT CAST(42 AS BIGINT)`) and
/// emits the first cell of the first result row.
///
/// Uses the FFI-safe `query()` API (returns SDK `Value`s) rather than
/// `query_arrow()`: arrow arrays produced by the runtime cannot be downcast in
/// UDF code because the `.so` links a separate copy of `arrow` with different
/// `TypeId`s. BIGINT arrives as `Value::Numeric` (decimal string), so we parse
/// it and re-emit it as Numeric (BIGINT EMITS columns travel as PB_NUMERIC).
#[exasol_udf]
pub fn connect_back_query(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    // Drain the single FROM DUAL input row before opening the connect-back session.
    while ctx.next()? {}
    let c = ctx.connection("CB_SELF")?;
    let mut conn = ctx.connect_back(&c)?;
    let rows = conn.query("SELECT CAST(42 AS BIGINT)")?;
    let cell = rows
        .first()
        .and_then(|r| r.first())
        .ok_or_else(|| UdfError::User("connect_back_query: empty result".into()))?;
    let val: i64 = match cell {
        Value::Int64(n) => *n,
        Value::Int32(n) => *n as i64,
        Value::Numeric(d) if d.scale == 0 => i64::try_from(d.unscaled)
            .map_err(|_| UdfError::Type(format!("Numeric value {} overflows i64", d)))?,
        other => return Err(UdfError::Type(format!("unexpected value {other:?}"))),
    };
    // BIGINT EMITS columns travel as PB_NUMERIC (typed Decimal).
    ctx.emit(&[Value::Numeric(Decimal {
        unscaled: val as i128,
        scale: 0,
    })])
}
