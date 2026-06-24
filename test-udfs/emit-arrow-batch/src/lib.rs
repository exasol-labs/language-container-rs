use std::sync::Arc;

use arrow::array::{Int64Array, StringArray};
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use exasol_udf_macros::exasol_udf;
use exasol_udf_sdk::context::{EmitBatch, UdfContext};
use exasol_udf_sdk::error::UdfError;

/// SET UDF: drains all input rows then emits a fixed 3-row RecordBatch.
///
/// Emitted rows (in order):
///   id=1, label="a"
///   id=2, label="b"
///   id=3, label="c"
///
/// EMITS schema: `id BIGINT, label VARCHAR(1)`
#[exasol_udf]
pub fn emit_arrow_batch(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    while ctx.next()? {}

    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("label", DataType::Utf8, false),
    ]));
    let ids = Arc::new(Int64Array::from(vec![1i64, 2, 3]));
    let labels = Arc::new(StringArray::from(vec!["a", "b", "c"]));
    let batch = RecordBatch::try_new(schema, vec![ids, labels])
        .map_err(|e| UdfError::User(e.to_string()))?;

    ctx.emit_batch(&batch)
}
