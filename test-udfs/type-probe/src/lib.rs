use exasol_udf_macros::exasol_udf;
use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::error::UdfError;
use exasol_udf_sdk::value::Value;

fn variant_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "Null",
        Value::Bool(_) => "Bool",
        Value::Int32(_) => "Int32",
        Value::Int64(_) => "Int64",
        Value::Double(_) => "Double",
        Value::String(_) => "String",
        Value::Numeric(_) => "Numeric",
        Value::Date(_) => "Date",
        Value::Timestamp(_) => "Timestamp",
    }
}

#[exasol_udf]
pub fn type_probe(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    let n_in = ctx.num_columns();
    let n_out = ctx.output_column_count();
    while ctx.next()? {
        let mut diag_parts = Vec::with_capacity(n_in);
        let mut values = Vec::with_capacity(n_out);
        for col in 0..n_in {
            let v = ctx.get(col)?;
            let info = ctx.input_column(col)?;
            diag_parts.push(format!(
                "{}:{}:{}:p={:?}:s={:?}",
                info.name,
                variant_name(v),
                info.type_name,
                info.precision,
                info.scale,
            ));
            values.push(v.clone());
        }
        values.push(Value::String(diag_parts.join(",")));
        ctx.emit(values)?;
    }
    Ok(())
}
