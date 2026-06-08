use exasol_udf_macros::exasol_udf;
use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::error::UdfError;
use exasol_udf_sdk::value::Value;

/// SET UDF that creates `cb_result` (if needed) and inserts each input row's
/// value into it, then emits a row-count of 1 per inserted row.
#[exasol_udf]
pub fn connect_back_insert(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    ctx.connect_back(&ctx.connection("CB_SELF")?)?
        .execute("CREATE TABLE IF NOT EXISTS cb_result (val BIGINT)")?;
    while ctx.next()? {
        let val = match ctx.get(0)? {
            Value::Int64(n) => *n,
            Value::Numeric(s) => s.parse().map_err(|e| UdfError::Type(format!("{e}")))?,
            _ => return Err(UdfError::Type("expected integer".into())),
        };
        ctx.connect_back(&ctx.connection("CB_SELF")?)?
            .execute(&format!("INSERT INTO cb_result VALUES ({val})"))?;
        ctx.emit(&[Value::Int64(1)])?;
    }
    Ok(())
}
