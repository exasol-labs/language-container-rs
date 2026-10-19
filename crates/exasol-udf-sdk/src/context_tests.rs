use super::*;
use crate::test_support::{DefaultsCtx, TestContext};

struct DummyUdf;

impl UdfRun for DummyUdf {
    fn run(_ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
        Ok(())
    }
}

#[test]
fn typed_accessors_read_the_current_row() {
    let date = chrono::NaiveDate::from_ymd_opt(2026, 6, 14).unwrap();
    let ctx = TestContext::scalar(vec![
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
    ]);

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
    let ctx = DefaultsCtx;
    assert_eq!(ctx.memory_limit(), 0);
}

#[test]
fn default_set_return_unimplemented() {
    let mut ctx = DefaultsCtx;
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
    let ctx = DefaultsCtx;
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
fn rows_in_group_defaults_to_zero() {
    let ctx = DefaultsCtx;
    assert_eq!(ctx.rows_in_group(), 0);
}

#[test]
fn iteration_axis_accessors_default_to_none() {
    let ctx = DefaultsCtx;
    assert_eq!(ctx.input_type(), None);
    assert_eq!(ctx.output_type(), None);
}

#[test]
fn default_debug_level_is_info() {
    let ctx = DefaultsCtx;
    assert_eq!(ctx.debug_level(), tracing::Level::INFO);
}

#[test]
fn udf_run_spec_hooks_default_to_unimplemented() {
    let mut ctx = DefaultsCtx;

    let vsa = DummyUdf::virtual_schema_adapter_call(&mut ctx, "{}");
    assert!(matches!(vsa, Err(UdfError::Unimplemented(_))));

    let doc = DummyUdf::default_output_columns();
    assert!(matches!(doc, Err(UdfError::Unimplemented(_))));

    let import = DummyUdf::generate_sql_for_import_spec(&mut ctx, "{}");
    assert!(matches!(import, Err(UdfError::Unimplemented(_))));

    let export = DummyUdf::generate_sql_for_export_spec(&mut ctx, "{}");
    assert!(matches!(export, Err(UdfError::Unimplemented(_))));
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

    let mut ctx = DefaultsCtx;
    assert!(matches!(
        ctx.emit_batch(&batch),
        Err(UdfError::Unimplemented(_))
    ));
}

#[cfg(feature = "emit-arrow")]
#[test]
fn emit_batch_exports_the_batch_through_the_c_data_interface() {
    use super::EmitBatch;
    use arrow::array::{Int64Array, StringArray, StructArray};
    use arrow::datatypes::{DataType, Field, Schema};
    use arrow::ffi::{FFI_ArrowArray, FFI_ArrowSchema};
    use arrow::record_batch::RecordBatch;
    use std::sync::Arc;

    struct Importer {
        decoded: Vec<RecordBatch>,
    }
    impl UdfContext for Importer {
        fn input_column_count(&self) -> usize {
            0
        }
        fn get(&self, col: usize) -> Result<&Value, UdfError> {
            Err(UdfError::Type(format!("get({col}): no input columns")))
        }
        fn emit(&mut self, _values: Vec<Value>) -> Result<(), UdfError> {
            Ok(())
        }
        fn next(&mut self) -> Result<bool, UdfError> {
            Ok(false)
        }
        fn emit_record_batch_ffi(
            &mut self,
            array: *mut core::ffi::c_void,
            schema: *mut core::ffi::c_void,
        ) -> Result<(), UdfError> {
            let array = unsafe {
                std::ptr::replace(array.cast::<FFI_ArrowArray>(), FFI_ArrowArray::empty())
            };
            let schema = unsafe { &*schema.cast::<FFI_ArrowSchema>() };
            let data = unsafe { arrow::ffi::from_ffi(array, schema) }.unwrap();
            self.decoded
                .push(RecordBatch::from(StructArray::from(data)));
            Ok(())
        }
    }

    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Int64, false),
        Field::new("s", DataType::Utf8, true),
    ]));
    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(Int64Array::from(vec![1i64, 2])),
            Arc::new(StringArray::from(vec![Some("a"), None])),
        ],
    )
    .unwrap();
    let mut ctx = Importer {
        decoded: Vec::new(),
    };
    ctx.emit_batch(&batch).unwrap();
    ctx.emit_batch(&batch).unwrap();
    assert_eq!(ctx.decoded, vec![batch.clone(), batch]);
}
