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
    use exasol_udf_sdk::test_support::{NextPolicy, TestContext};

    #[test]
    fn next_in_scalar_context_errors() {
        let mut ctx =
            TestContext::scalar(vec![Value::Int64(1)]).with_next_policy(NextPolicy::Reject(
                UdfError::User("next() is not allowed in scalar context".into()),
            ));
        let err = scalar_next_illegal(&mut ctx).unwrap_err();
        assert!(matches!(err, UdfError::User(msg) if msg.contains("scalar")));
    }
}
