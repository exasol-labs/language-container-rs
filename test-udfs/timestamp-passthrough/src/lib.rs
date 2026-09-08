use exasol_udf_macros::exasol_udf;
use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::error::UdfError;
use exasol_udf_sdk::value::Value;

#[exasol_udf]
pub fn timestamp_passthrough(
    ctx: &mut dyn UdfContext,
) -> Result<Option<chrono::NaiveDateTime>, UdfError> {
    let result = match ctx.get(0)? {
        Value::Timestamp(ts) => *ts,
        Value::Null => return Ok(None),
        other => {
            return Err(UdfError::Type(format!(
                "expected timestamp, got {:?}",
                other
            )));
        }
    };
    Ok(Some(result))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use exasol_udf_sdk::test_support::{EmitPolicy, TestContext};

    fn returns_ctx(row: Vec<Value>) -> TestContext {
        TestContext::scalar(row).with_emit_policy(EmitPolicy::Reject(UdfError::Unimplemented(
            "emit is banned in RETURNS output".into(),
        )))
    }

    #[test]
    fn passes_nanosecond_timestamp_through() {
        let input = NaiveDate::from_ymd_opt(2026, 6, 14)
            .unwrap()
            .and_hms_nano_opt(9, 30, 15, 123_456_789)
            .unwrap();

        let mut ctx = returns_ctx(vec![Value::Timestamp(input)]);
        let result = timestamp_passthrough(&mut ctx).unwrap();
        assert_eq!(result, Some(input));
    }

    #[test]
    fn passes_null_through() {
        let mut ctx = returns_ctx(vec![Value::Null]);
        let result = timestamp_passthrough(&mut ctx).unwrap();
        assert_eq!(result, None);
    }
}
