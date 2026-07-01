use exasol_udf_macros::exasol_udf;
use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::error::UdfError;
use exasol_udf_sdk::value::Value;

/// Scalar UDF that emits live handshake metadata so a DB round-trip can prove
/// the DB-supplied `exascript_info` values reach UDF code through the
/// `UdfContext` accessors. Emits a single pipe-delimited string:
/// `session_id|node_id|node_count|script_name`. Reads metadata only; opens no
/// connect-back session.
#[exasol_udf]
pub fn handshake_meta(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    let summary = format!(
        "{}|{}|{}|{}",
        ctx.session_id(),
        ctx.node_id(),
        ctx.node_count(),
        ctx.script_name(),
    );
    ctx.emit(&[Value::String(summary)])
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MetaCtx {
        session_id: u64,
        node_id: u32,
        node_count: u32,
        script_name: String,
        emitted: Vec<Vec<Value>>,
    }

    impl UdfContext for MetaCtx {
        fn num_columns(&self) -> usize {
            0
        }

        fn get(&self, _col: usize) -> Result<&Value, UdfError> {
            Err(UdfError::Type("no input columns".into()))
        }

        fn emit(&mut self, values: &[Value]) -> Result<(), UdfError> {
            self.emitted.push(values.to_vec());
            Ok(())
        }

        fn next(&mut self) -> Result<bool, UdfError> {
            Ok(false)
        }

        fn session_id(&self) -> u64 {
            self.session_id
        }

        fn node_id(&self) -> u32 {
            self.node_id
        }

        fn node_count(&self) -> u32 {
            self.node_count
        }

        fn script_name(&self) -> String {
            self.script_name.clone()
        }
    }

    #[test]
    fn emits_pipe_delimited_handshake_summary() {
        let mut ctx = MetaCtx {
            session_id: 1_700_000_000_000_123,
            node_id: 0,
            node_count: 1,
            script_name: "handshake_meta".into(),
            emitted: Vec::new(),
        };
        handshake_meta(&mut ctx).unwrap();
        assert_eq!(
            ctx.emitted,
            vec![vec![Value::String(
                "1700000000000123|0|1|handshake_meta".into()
            )]]
        );
    }
}
