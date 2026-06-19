use exasol_udf_macros::exasol_udf;
use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::error::UdfError;
use exasol_udf_sdk::value::Value;

#[exasol_udf]
pub fn timestamp_passthrough(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    let result = match ctx.get(0)? {
        Value::Timestamp(ts) => Value::Timestamp(*ts),
        Value::Null => Value::Null,
        other => {
            return Err(UdfError::Type(format!(
                "expected timestamp, got {:?}",
                other
            )));
        }
    };
    ctx.emit(&[result])
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    struct TestCtx {
        input: Vec<Value>,
        emitted: Vec<Vec<Value>>,
    }

    impl TestCtx {
        fn new(row: Vec<Value>) -> Self {
            Self {
                input: row,
                emitted: Vec::new(),
            }
        }
    }

    impl UdfContext for TestCtx {
        fn num_columns(&self) -> usize {
            self.input.len()
        }

        fn get(&self, col: usize) -> Result<&Value, UdfError> {
            self.input
                .get(col)
                .ok_or_else(|| UdfError::User(format!("col {} out of range", col)))
        }

        fn emit(&mut self, values: &[Value]) -> Result<(), UdfError> {
            self.emitted.push(values.to_vec());
            Ok(())
        }

        fn next(&mut self) -> Result<bool, UdfError> {
            Ok(false)
        }
    }

    #[test]
    fn passes_nanosecond_timestamp_through() {
        let input = NaiveDate::from_ymd_opt(2026, 6, 14)
            .unwrap()
            .and_hms_nano_opt(9, 30, 15, 123_456_789)
            .unwrap();

        let mut ctx = TestCtx::new(vec![Value::Timestamp(input)]);
        timestamp_passthrough(&mut ctx).unwrap();
        assert_eq!(ctx.emitted, vec![vec![Value::Timestamp(input)]]);
    }

    #[test]
    fn passes_null_through() {
        let mut ctx = TestCtx::new(vec![Value::Null]);
        timestamp_passthrough(&mut ctx).unwrap();
        assert_eq!(ctx.emitted, vec![vec![Value::Null]]);
    }
}
