use arrow::array::Int64Array;
use exasol_udf_macros::exasol_udf;
use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::error::UdfError;
use exasol_udf_sdk::value::Value;

/// Scalar UDF that issues a connect-back query (`SELECT 42`) and emits the
/// first cell of the first result batch.
#[exasol_udf]
pub fn connect_back_query(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    let c = ctx.connection("CB_SELF")?;
    let batches = ctx.connect_back(&c)?.query_arrow("SELECT 42")?;
    let first_val = batches
        .first()
        .and_then(|b| b.column(0).as_any().downcast_ref::<Int64Array>())
        .map(|a| a.value(0))
        .unwrap_or(0);
    ctx.emit(&[Value::Int64(first_val)])
}
