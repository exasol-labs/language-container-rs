use crate::error::UdfError;
use crate::value::Value;

/// Context for a single UDF call — provided by the host, read by the UDF
pub trait UdfContext {
    /// Number of input columns
    fn num_columns(&self) -> usize;
    /// Get a specific input column value (0-indexed)
    fn get(&self, col: usize) -> Result<&Value, UdfError>;
    /// Emit one output row. For scalar: called once per input row. For set/EMITS: called per output row.
    fn emit(&mut self, values: &[Value]) -> Result<(), UdfError>;
    /// Advance to the next input row (set UDFs only). Returns false when exhausted.
    fn next(&mut self) -> Result<bool, UdfError>;
}

/// Per-call lifecycle hooks — default implementations return Unimplemented for v1 single-call hooks
pub trait UdfRun: Sized {
    fn run(ctx: &mut dyn UdfContext) -> Result<(), UdfError>;

    /// Called once before run() — default: Unimplemented
    fn virtual_schema_adapter_call(
        _ctx: &mut dyn UdfContext,
        _json_arg: &str,
    ) -> Result<String, UdfError> {
        Err(UdfError::Unimplemented(
            "virtual_schema_adapter_call".into(),
        ))
    }
    /// Called once before run() — default: Unimplemented
    fn default_output_columns() -> Result<String, UdfError> {
        Err(UdfError::Unimplemented("default_output_columns".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct DummyCtx;

    impl UdfContext for DummyCtx {
        fn num_columns(&self) -> usize {
            0
        }
        fn get(&self, _col: usize) -> Result<&Value, UdfError> {
            Err(UdfError::Type("no columns".into()))
        }
        fn emit(&mut self, _values: &[Value]) -> Result<(), UdfError> {
            Ok(())
        }
        fn next(&mut self) -> Result<bool, UdfError> {
            Ok(false)
        }
    }

    struct DummyUdf;

    impl UdfRun for DummyUdf {
        fn run(_ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
            Ok(())
        }
    }

    #[test]
    fn default_hooks_unimplemented() {
        let mut ctx = DummyCtx;

        let vsa = DummyUdf::virtual_schema_adapter_call(&mut ctx, "{}");
        assert!(matches!(vsa, Err(UdfError::Unimplemented(_))));

        let doc = DummyUdf::default_output_columns();
        assert!(matches!(doc, Err(UdfError::Unimplemented(_))));
    }
}
