use exasol_udf_macros::exasol_udf;
use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::error::UdfError;
use exasol_udf_sdk::value::Value;

/// SCALAR RETURNS fixture whose body also calls `ctx.emit()` before returning
/// a value. `emit()` is a legal call at compile time; the runtime bans it at
/// call time for RETURNS-shape UDFs (Bug 3 guard), so this fixture must
/// compile cleanly and let the ban be exercised at IT time.
#[exasol_udf]
pub fn returns_with_emit(ctx: &mut dyn UdfContext) -> Result<Option<Value>, UdfError> {
    ctx.emit(&[Value::Int64(1)])?;
    Ok(Some(Value::Int64(42)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use exasol_udf_sdk::test_support::TestContext;

    #[test]
    fn calls_emit_then_returns_a_value() {
        let mut ctx = TestContext::scalar(vec![]);
        let result = returns_with_emit(&mut ctx).unwrap();
        assert_eq!(result, Some(Value::Int64(42)));
        assert_eq!(ctx.emitted(), vec![vec![Value::Int64(1)]]);
    }
}
