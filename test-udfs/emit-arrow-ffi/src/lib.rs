use std::sync::Arc;

use arrow::array::{Array, Int64Array, StringArray, StructArray};
use arrow::datatypes::{DataType, Field, Schema};
use arrow::ffi::{FFI_ArrowArray, FFI_ArrowSchema};
use arrow::record_batch::RecordBatch;
use exasol_udf_macros::exasol_udf;
use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::error::UdfError;

/// SET UDF emitting the `emit-arrow-batch` rows through `emit_record_batch_ffi`,
/// with its own arrow and `emit-arrow` off. EMITS `id BIGINT, label VARCHAR(1)`.
#[exasol_udf]
pub fn emit_arrow_ffi(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    while ctx.next()? {}

    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("label", DataType::Utf8, false),
    ]));
    let ids = Arc::new(Int64Array::from(vec![1i64, 2, 3]));
    let labels = Arc::new(StringArray::from(vec!["a", "b", "c"]));
    let batch = RecordBatch::try_new(schema, vec![ids, labels])
        .map_err(|e| UdfError::User(e.to_string()))?;

    let data = StructArray::from(batch).into_data();
    let (mut array, mut schema) =
        arrow::ffi::to_ffi(&data).map_err(|e| UdfError::Type(e.to_string()))?;
    ctx.emit_record_batch_ffi(
        (&mut array as *mut FFI_ArrowArray).cast(),
        (&mut schema as *mut FFI_ArrowSchema).cast(),
    )
}
