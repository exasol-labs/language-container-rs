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
    use exasol_udf_sdk::test_support::TestContext;

    #[test]
    fn emits_zero_rows_for_zero_count() {
        let mut ctx = TestContext::scalar(vec![Value::Int64(0)]);
        emit_k(&mut ctx).unwrap();
        assert!(ctx.emitted().is_empty());
    }

    #[test]
    fn emits_one_row_for_count_one() {
        let mut ctx = TestContext::scalar(vec![Value::Int64(1)]);
        emit_k(&mut ctx).unwrap();
        assert_eq!(ctx.emitted(), vec![vec![Value::Int64(0)]]);
    }

    #[test]
    fn emits_n_rows_for_count_n() {
        let mut ctx = TestContext::scalar(vec![Value::Int64(4)]);
        emit_k(&mut ctx).unwrap();
        assert_eq!(
            ctx.emitted(),
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
        let mut ctx = TestContext::scalar(vec![Value::Null]);
        emit_k(&mut ctx).unwrap();
        assert!(ctx.emitted().is_empty());
    }
}
