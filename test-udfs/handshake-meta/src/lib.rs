use exasol_udf_macros::exasol_udf;
use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::error::UdfError;

/// Scalar UDF that returns live handshake metadata so a DB round-trip can prove
/// the DB-supplied `exascript_info` values reach UDF code through the
/// `UdfContext` accessors. Returns a single pipe-delimited string:
/// `session_id|node_id|node_count|script_name`. Reads metadata only; opens no
/// connect-back session.
#[exasol_udf]
pub fn handshake_meta(ctx: &mut dyn UdfContext) -> Result<Option<String>, UdfError> {
    let summary = format!(
        "{}|{}|{}|{}",
        ctx.session_id(),
        ctx.node_id(),
        ctx.node_count(),
        ctx.script_name(),
    );
    Ok(Some(summary))
}

#[cfg(test)]
mod tests {
    use super::*;
    use exasol_udf_sdk::test_support::{EmitPolicy, TestContext};
    use exasol_udf_sdk::value::Value;

    fn returns_ctx(row: Vec<Value>) -> TestContext {
        TestContext::scalar(row).with_emit_policy(EmitPolicy::Reject(UdfError::Unimplemented(
            "emit is banned in RETURNS output".into(),
        )))
    }

    #[test]
    fn returns_pipe_delimited_handshake_summary() {
        let mut ctx = returns_ctx(vec![])
            .with_session_id(1_700_000_000_000_123)
            .with_node_id(0)
            .with_node_count(1)
            .with_script_name("handshake_meta");
        let result = handshake_meta(&mut ctx).unwrap();
        assert_eq!(result, Some("1700000000000123|0|1|handshake_meta".into()));
    }
}
