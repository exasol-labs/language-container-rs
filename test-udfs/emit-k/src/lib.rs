use exasol_udf_macros::exasol_udf;
use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::error::UdfError;
use exasol_udf_sdk::value::Value;

/// SCALAR EMITS fixture: reads the first column as a row count `k` and emits
/// `k` rows (the emitted value is the 0-based row index), so a single scalar
/// invocation can produce 0, 1, or many output rows.
#[exasol_udf]
pub fn emit_k(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    let k = ctx.get_i64(0)?.unwrap_or(0);
    for i in 0..k {
        ctx.emit(&[Value::Int64(i)])?;
    }
    Ok(())
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
    fn emits_zero_rows_for_zero_count() {
        let mut ctx = TestCtx::new(vec![Value::Int64(0)]);
        emit_k(&mut ctx).unwrap();
        assert!(ctx.emitted.is_empty());
    }

    #[test]
    fn emits_one_row_for_count_one() {
        let mut ctx = TestCtx::new(vec![Value::Int64(1)]);
        emit_k(&mut ctx).unwrap();
        assert_eq!(ctx.emitted, vec![vec![Value::Int64(0)]]);
    }

    #[test]
    fn emits_n_rows_for_count_n() {
        let mut ctx = TestCtx::new(vec![Value::Int64(4)]);
        emit_k(&mut ctx).unwrap();
        assert_eq!(
            ctx.emitted,
            vec![
                vec![Value::Int64(0)],
                vec![Value::Int64(1)],
                vec![Value::Int64(2)],
                vec![Value::Int64(3)],
            ]
        );
    }

    #[test]
    fn null_count_emits_nothing() {
        let mut ctx = TestCtx::new(vec![Value::Null]);
        emit_k(&mut ctx).unwrap();
        assert!(ctx.emitted.is_empty());
    }
}
