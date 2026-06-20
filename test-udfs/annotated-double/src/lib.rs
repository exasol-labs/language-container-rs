use exasol_udf_macros::exasol_udf;
use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::error::UdfError;
use exasol_udf_sdk::value::{Decimal, Value};

/// Scalar UDF that doubles its i64 input, with annotated schema metadata.
#[exasol_udf(input(x: Decimal), emits(result: Decimal))]
pub fn annotated_double(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    let v = match ctx.get(0)? {
        Value::Int64(n) => Value::Int64(n * 2),
        Value::Numeric(d) if d.scale == 0 => {
            let n = i64::try_from(d.unscaled)
                .map_err(|_| UdfError::Type(format!("Numeric value {} overflows i64", d)))?;
            Value::Numeric(Decimal {
                unscaled: (n * 2) as i128,
                scale: 0,
            })
        }
        _ => return Err(UdfError::Type("expected i64".into())),
    };
    ctx.emit(&[v])
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
                .ok_or_else(|| UdfError::User(format!("col {col} out of range")))
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
    fn doubles_int64() {
        let mut ctx = TestCtx::new(vec![Value::Int64(21)]);
        annotated_double(&mut ctx).unwrap();
        assert_eq!(ctx.emitted, vec![vec![Value::Int64(42)]]);
    }

    #[test]
    fn schema_pointers_non_null() {
        let vtable = __exa_udf_entry_ANNOTATED_DOUBLE();
        let vt = unsafe { &*vtable };
        assert!(!vt.annotated_input_schema.is_null());
        assert!(!vt.annotated_output_schema.is_null());
    }
}
