use exasol_udf_macros::exasol_udf;
use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::error::UdfError;
use exasol_udf_sdk::value::{Decimal, Value};

/// SCALAR UDF that issues a connect-back query (`SELECT CAST(42 AS BIGINT)`) and
/// returns the first cell of the first result row.
///
/// Identical connect-back logic to the `connect-back-query` SET UDF — the only
/// difference is registration (`RUST SCALAR SCRIPT ... RETURNS BIGINT` vs
/// `SET SCRIPT ... EMITS`). A scalar UDF surfaces its single value through the
/// RETURNS channel (`Ok(Some(v))`, which the runtime forwards via `set_return`;
/// `Ok(None)` yields SQL NULL). This proves connect-back works from a SCALAR
/// script, not just SET/EMITS.
#[exasol_udf]
pub fn connect_back_scalar(ctx: &mut dyn UdfContext) -> Result<Option<Decimal>, UdfError> {
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
    Ok(Some(Decimal {
        unscaled: val as i128,
        scale: 0,
    }))
}

/// Looks up the comma-separated CONNECTION names of the first argument in
/// order and returns the results joined by `|`. On a failed lookup the second
/// argument picks the reaction: `propagate` returns the lookup error, `catch`
/// records its text, `unwrap` panics, anything else becomes the UDF's own error
/// text. `panic` panics before any lookup.
#[exasol_udf]
pub fn connection_lookup(ctx: &mut dyn UdfContext) -> Result<Option<String>, UdfError> {
    let mut args = (0..2).map(|i| match ctx.get(i)? {
        Value::String(s) => Ok(s.clone()),
        Value::Null => Ok(String::new()),
        other => Err(UdfError::Type(format!("expected VARCHAR, got {other:?}"))),
    });
    let (names, on_error) = (args.next().unwrap()?, args.next().unwrap()?);
    if on_error == "panic" {
        panic!("connection_lookup panicked without a lookup");
    }
    let mut results = Vec::new();
    for name in names.split(',') {
        let looked_up = match on_error.as_str() {
            "unwrap" => Ok(ctx.connection(name).unwrap()),
            _ => ctx.connection(name),
        };
        results.push(match (looked_up, on_error.as_str()) {
            (Ok(c), _) => c.address,
            (Err(e), "propagate") => return Err(e),
            (Err(e), "catch") => e.to_string(),
            (Err(_), own) => return Err(UdfError::User(own.into())),
        });
    }
    Ok(Some(results.join("|")))
}
