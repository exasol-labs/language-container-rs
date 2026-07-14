use exasol_udf_macros::exasol_udf;
use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::error::UdfError;
#[cfg(test)]
use exasol_udf_sdk::value::Value;

/// SCALAR fixture that calls the banned `ctx.next()` in scalar input context.
/// Registered `SCALAR` at IT time, this must trigger the runtime's
/// `F-UDF-CL-RUST-` next-in-scalar gate (Bug 3 guard) rather than run to
/// completion.
#[exasol_udf]
pub fn scalar_next_illegal(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    ctx.next()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mimics the runtime's scalar-input gate: `next()` always errors.
    struct TestCtx {
        input: Vec<Value>,
    }

    impl TestCtx {
        fn new(row: Vec<Value>) -> Self {
            Self { input: row }
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

        fn emit(&mut self, _values: &[Value]) -> Result<(), UdfError> {
            Ok(())
        }

        fn next(&mut self) -> Result<bool, UdfError> {
            Err(UdfError::User(
                "next() is not allowed in scalar context".into(),
            ))
        }
    }

    #[test]
    fn next_in_scalar_context_errors() {
        let mut ctx = TestCtx::new(vec![Value::Int64(1)]);
        let err = scalar_next_illegal(&mut ctx).unwrap_err();
        assert!(matches!(err, UdfError::User(msg) if msg.contains("scalar")));
    }
}
