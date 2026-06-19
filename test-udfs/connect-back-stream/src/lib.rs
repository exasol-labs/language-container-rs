use exasol_udf_macros::exasol_udf;
use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::error::UdfError;
use exasol_udf_sdk::value::{Decimal, Value};

#[exasol_udf]
pub fn connect_back_stream(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    while ctx.next()? {}
    let c = ctx.connection("CB_SELF")?;
    let mut conn = ctx.connect_back(&c)?;
    let mut count: i64 = 0;
    conn.query_for_each("SELECT 1 FROM it_rust.cb_stream_seed", &mut |_row| {
        count += 1;
        Ok(())
    })?;
    ctx.emit(&[Value::Numeric(Decimal {
        unscaled: count as i128,
        scale: 0,
    })])
}
