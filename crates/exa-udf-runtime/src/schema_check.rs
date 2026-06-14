use crate::error::RuntimeError;
use crate::loader::LoadedUdf;
use exa_zmq_protocol::{ColumnMeta, ExaType, UdfMeta};

/// One field of an annotated schema as embedded in the vtable by the
/// `#[exasol_udf]` macro: `{"name":"x","type":"Int64"}`.
struct AnnotatedField {
    name: String,
    typ: String,
}

/// Validate the UDF's annotated input/output schema (if present) against the
/// column metadata the database sent during the handshake.
///
/// Each annotated schema is optional: a UDF that was not annotated with
/// `input(...)` / `emits(...)` exposes a null pointer and is accepted as-is
/// (the DB metadata is authoritative). When a schema *is* annotated, the column
/// count, names, and types must match exactly, in order. Any divergence is a
/// hard error so the session is closed before any rows are processed.
pub fn validate_schema(udf: &LoadedUdf, meta: &UdfMeta) -> Result<(), RuntimeError> {
    if let Some(json) = udf.annotated_input_schema() {
        validate_one("input", json, &meta.input_columns)?;
    }
    if let Some(json) = udf.annotated_output_schema() {
        validate_one("output", json, &meta.output_columns)?;
    }
    Ok(())
}

fn validate_one(side: &str, json: &str, columns: &[ColumnMeta]) -> Result<(), RuntimeError> {
    let annotated = parse_schema(side, json)?;

    if annotated.len() != columns.len() {
        return Err(RuntimeError::Udf(format!(
            "annotated {side} schema has {} column(s) but the database supplied {}",
            annotated.len(),
            columns.len()
        )));
    }

    for (idx, (field, col)) in annotated.iter().zip(columns.iter()).enumerate() {
        if field.name != col.name {
            return Err(RuntimeError::Udf(format!(
                "annotated {side} column {idx} is named `{}` but the database supplied `{}`",
                field.name, col.name
            )));
        }
        let expected = exatype_name(&col.typ);
        if field.typ != expected {
            return Err(RuntimeError::Udf(format!(
                "annotated {side} column `{}` is typed `{}` but the database supplied `{}`",
                field.name, field.typ, expected
            )));
        }
    }
    Ok(())
}

/// Parse the macro-emitted schema JSON array into typed fields.
fn parse_schema(side: &str, json: &str) -> Result<Vec<AnnotatedField>, RuntimeError> {
    let value: serde_json::Value = serde_json::from_str(json)
        .map_err(|e| RuntimeError::Udf(format!("invalid annotated {side} schema JSON: {e}")))?;
    let array = value
        .as_array()
        .ok_or_else(|| RuntimeError::Udf(format!("annotated {side} schema is not a JSON array")))?;
    let mut fields = Vec::with_capacity(array.len());
    for entry in array {
        let name = entry
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                RuntimeError::Udf(format!("annotated {side} schema entry missing `name`"))
            })?
            .to_string();
        let typ = entry
            .get("type")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                RuntimeError::Udf(format!("annotated {side} schema entry missing `type`"))
            })?
            .to_string();
        fields.push(AnnotatedField { name, typ });
    }
    Ok(fields)
}

/// The annotation type name the `#[exasol_udf]` macro emits for an [`ExaType`].
/// Must stay in lockstep with `exasol-udf-macros::rust_type_to_exatype`.
fn exatype_name(typ: &ExaType) -> &'static str {
    match typ {
        ExaType::Int32 => "Int32",
        ExaType::Int64 => "Int64",
        ExaType::Numeric { .. } => "Numeric",
        ExaType::Double => "Double",
        ExaType::Boolean => "Boolean",
        ExaType::Date => "Date",
        ExaType::Timestamp | ExaType::TimestampTz => "Timestamp",
        ExaType::String { .. }
        | ExaType::Char { .. }
        | ExaType::Geometry
        | ExaType::HashType
        | ExaType::IntervalYearToMonth
        | ExaType::IntervalDayToSecond => "String",
        ExaType::Unsupported => "Unsupported",
    }
}
