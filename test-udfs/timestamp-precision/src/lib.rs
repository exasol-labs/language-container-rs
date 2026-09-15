use exasol_udf_macros::exasol_udf;
use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::error::UdfError;
use exasol_udf_sdk::value::{ExaType, Value};

#[exasol_udf]
pub fn ts_precision_probe(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    let ts_col = ctx.output_column(0)?;
    let precision = match ts_col.typ {
        ExaType::Timestamp { precision } | ExaType::TimestampTz { precision } => precision,
        _ => return Err(UdfError::Type("output column 0 is not a timestamp".into())),
    };
    let prec_col = ctx.output_column(1)?;
    if !matches!(
        prec_col.typ,
        ExaType::Int32 | ExaType::Int64 | ExaType::Numeric { .. }
    ) {
        return Err(UdfError::Type("output column 1 is not numeric".into()));
    }

    let ts = chrono::NaiveDate::from_ymd_opt(2026, 7, 14)
        .unwrap()
        .and_hms_nano_opt(9, 30, 15, 123_456_789)
        .unwrap();
    ctx.emit(vec![Value::Timestamp(ts), Value::Int32(precision as i32)])
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
