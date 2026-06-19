use exasol_udf_macros::exasol_udf;
use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::error::UdfError;
use exasol_udf_sdk::value::Value;

const PAYLOAD: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"; // 98 chars + overhead

#[exasol_udf]
pub fn emit_bulk(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    // Read repeat count from first column
    let n: i64 = match ctx.next()? {
        false => return Ok(()),
        true => match ctx.get(0)? {
            Value::Int64(n) => *n,
            Value::Numeric(d) if d.scale == 0 => {
                i64::try_from(d.unscaled).map_err(|_| UdfError::Type("n overflow".into()))?
            }
            other => return Err(UdfError::Type(format!("expected BIGINT, got {other:?}"))),
        },
    };
    while ctx.next()? {} // drain remaining input rows
    for _ in 0..n {
        ctx.emit(&[Value::String(PAYLOAD.to_string())])?;
    }
    Ok(())
}
