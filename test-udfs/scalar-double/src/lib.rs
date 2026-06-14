use exasol_udf_macros::exasol_udf;
use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::error::UdfError;
use exasol_udf_sdk::value::{Decimal, Value};

#[exasol_udf]
pub fn scalar_double(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    let doubled = match ctx.get(0)? {
        Value::Int64(n) => Value::Int64(n * 2),
        // Exasol sends BIGINT as PB_NUMERIC (typed Decimal with scale=0).
        Value::Numeric(d) if d.scale == 0 => {
            let n = i64::try_from(d.unscaled)
                .map_err(|_| UdfError::Type(format!("Numeric value {} overflows i64", d)))?;
            Value::Numeric(Decimal {
                unscaled: (n * 2) as i128,
                scale: 0,
            })
        }
        Value::Null => Value::Null,
        _ => return Err(UdfError::Type("expected Int64 or Numeric".into())),
    };
    ctx.emit(&[doubled])
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn doubles_positive_int64() {
        let mut ctx = TestCtx::new(vec![Value::Int64(21)]);
        scalar_double(&mut ctx).unwrap();
        assert_eq!(ctx.emitted, vec![vec![Value::Int64(42)]]);
    }

    #[test]
    fn doubles_negative_int64() {
        let mut ctx = TestCtx::new(vec![Value::Int64(-5)]);
        scalar_double(&mut ctx).unwrap();
        assert_eq!(ctx.emitted, vec![vec![Value::Int64(-10)]]);
    }

    #[test]
    fn passes_null_through() {
        let mut ctx = TestCtx::new(vec![Value::Null]);
        scalar_double(&mut ctx).unwrap();
        assert_eq!(ctx.emitted, vec![vec![Value::Null]]);
    }

    #[test]
    fn rejects_wrong_type() {
        let mut ctx = TestCtx::new(vec![Value::String("x".into())]);
        let err = scalar_double(&mut ctx).unwrap_err();
        assert!(matches!(err, UdfError::Type(_)));
    }
}
