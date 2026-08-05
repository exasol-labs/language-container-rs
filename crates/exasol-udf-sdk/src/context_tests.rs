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

struct TypedDummyCtx {
    values: Vec<Value>,
}

impl UdfContext for TypedDummyCtx {
    fn num_columns(&self) -> usize {
        self.values.len()
    }
    fn get(&self, col: usize) -> Result<&Value, UdfError> {
        self.values
            .get(col)
            .ok_or_else(|| UdfError::Type("out of range".into()))
    }
    fn emit(&mut self, _values: &[Value]) -> Result<(), UdfError> {
        Ok(())
    }
    fn next(&mut self) -> Result<bool, UdfError> {
        Ok(false)
    }
}

#[test]
fn bridge_typed_getters_return_typed_options() {
    let date = chrono::NaiveDate::from_ymd_opt(2026, 6, 14).unwrap();
    let ctx = TypedDummyCtx {
        values: vec![
            Value::Int64(42),
            Value::Numeric(Decimal {
                unscaled: 100,
                scale: 0,
            }),
            Value::Numeric(Decimal {
                unscaled: 15,
                scale: 1,
            }),
            Value::Date(date),
            Value::Null,
            Value::Int64(1),
        ],
    };

    assert_eq!(ctx.get_i64(0).unwrap(), Some(42));
    assert_eq!(ctx.get_i64(1).unwrap(), Some(100));
    assert!(matches!(ctx.get_i64(2), Err(UdfError::Type(_))));

    let expected = Decimal {
        unscaled: 15,
        scale: 1,
    };
    assert_eq!(ctx.get_decimal(2).unwrap(), Some(expected));

    assert_eq!(ctx.get_date(3).unwrap(), Some(date));
    assert_eq!(ctx.get_value(4).unwrap(), None);
    assert!(matches!(ctx.get_f64(5), Err(UdfError::Type(_))));
}

#[test]
fn default_memory_limit_is_zero() {
    let ctx = DummyCtx;
    assert_eq!(ctx.memory_limit(), 0);
}

#[test]
fn default_set_return_unimplemented() {
    let mut ctx = DummyCtx;
    assert!(matches!(
        ctx.set_return(Some(Value::Int64(1))),
        Err(UdfError::Unimplemented(_))
    ));
    assert!(matches!(
        ctx.set_return(None),
        Err(UdfError::Unimplemented(_))
    ));
}

#[test]
fn default_handshake_metadata_is_neutral() {
    let ctx = DummyCtx;
    // Numeric accessors default to 0 ("not reported").
    assert_eq!(ctx.session_id(), 0u64);
    assert_eq!(ctx.statement_id(), 0u32);
    assert_eq!(ctx.node_id(), 0u32);
    assert_eq!(ctx.node_count(), 0u32);
    assert_eq!(ctx.vm_id(), 0u64);
    // Owned-string accessors default to the empty string.
    assert_eq!(ctx.database_name(), "");
    assert_eq!(ctx.database_version(), "");
    assert_eq!(ctx.script_name(), "");
    assert_eq!(ctx.script_schema(), "");
    // Optional accessors default to None (mirroring proto `optional`).
    assert_eq!(ctx.current_user(), None);
    assert_eq!(ctx.current_schema(), None);
    assert_eq!(ctx.scope_user(), None);
}

#[test]
fn default_debug_level_is_info() {
    let ctx = DummyCtx;
    assert_eq!(ctx.debug_level(), tracing::Level::INFO);
}

#[test]
fn default_hooks_unimplemented() {
    let mut ctx = DummyCtx;

    let vsa = DummyUdf::virtual_schema_adapter_call(&mut ctx, "{}");
    assert!(matches!(vsa, Err(UdfError::Unimplemented(_))));

    let doc = DummyUdf::default_output_columns();
    assert!(matches!(doc, Err(UdfError::Unimplemented(_))));
}

#[cfg(feature = "emit-arrow")]
#[test]
fn default_emit_batch_unimplemented() {
    use super::EmitBatch;
    use arrow::array::Int64Array;
    use arrow::datatypes::{DataType, Field, Schema};
    use arrow::record_batch::RecordBatch;
    use std::sync::Arc;

    let schema = Arc::new(Schema::new(vec![Field::new("x", DataType::Int64, false)]));
    let array = Arc::new(Int64Array::from(vec![1i64]));
    let batch = RecordBatch::try_new(schema, vec![array]).unwrap();

    // `emit_batch` (the EmitBatch ext-trait) serialises to IPC then calls
    // the default `emit_record_batch_ipc`, which is unimplemented on a
    // context that does not override it.
    let mut ctx = DummyCtx;
    assert!(matches!(
        ctx.emit_batch(&batch),
        Err(UdfError::Unimplemented(_))
    ));
}
