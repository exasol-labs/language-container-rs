//! The `json_spec` payload the two spec-generation hooks receive.
//!
//! `import_specification_rep` and `export_specification_rep` are protobuf
//! messages, and protobuf types are not ABI-stable across the `.so` boundary,
//! so the dispatcher hands the hook JSON instead. This module is the single
//! owner of that shape: a mechanical 1:1 mirror of the proto message under the
//! proto field names, with every key emitted unconditionally (an absent
//! `optional` as `null`, a `repeated` as `[]`) so a consumer parses one shape
//! instead of branching on what the database populated.
//!
//! A `column_type` keeps its proto variant name, for example `"PB_DOUBLE"`:
//! `exa-zmq-protocol` owns the proto-to-`ExaType` mapping, and a second mapping
//! here would be a second place to change.
//!
//! The serialized text carries `connection_information`, which holds the
//! CONNECTION object's password, so it is never logged or traced.

use exa_proto::exascript_metadata::ColumnDefinition;
use exa_proto::{
    ColumnType, ConnectionInformationRep, ExportSpecificationRep, ImportSpecificationRep,
    KeyValuePair,
};
use serde_json::{Value, json};

/// Serialize the `IMPORT ... FROM SCRIPT` specification for its hook.
pub(crate) fn serialize_import(spec: &ImportSpecificationRep) -> String {
    json!({
        "is_subselect": spec.is_subselect,
        "connection_information": credentials(spec.connection_information.as_ref()),
        "connection_name": spec.connection_name,
        "subselect_column_specification": Value::Array(
            spec.subselect_column_specification.iter().map(column).collect(),
        ),
        "parameters": parameters(&spec.parameters),
    })
    .to_string()
}

/// Serialize the `EXPORT ... INTO SCRIPT` specification for its hook.
pub(crate) fn serialize_export(spec: &ExportSpecificationRep) -> String {
    json!({
        "has_truncate": spec.has_truncate,
        "has_replace": spec.has_replace,
        "created_by": spec.created_by,
        "source_column_names": spec.source_column_names,
        "connection_information": credentials(spec.connection_information.as_ref()),
        "connection_name": spec.connection_name,
        "parameters": parameters(&spec.parameters),
    })
    .to_string()
}

fn credentials(conn: Option<&ConnectionInformationRep>) -> Value {
    match conn {
        Some(conn) => json!({
            "kind": conn.kind,
            "address": conn.address,
            "user": conn.user,
            "password": conn.password,
        }),
        None => Value::Null,
    }
}

fn parameters(pairs: &[KeyValuePair]) -> Value {
    Value::Array(
        pairs
            .iter()
            .map(|pair| json!({ "key": pair.key, "value": pair.value }))
            .collect(),
    )
}

fn column(def: &ColumnDefinition) -> Value {
    json!({
        "name": def.name,
        "type": def
            .r#type
            .and_then(|code| ColumnType::try_from(code).ok())
            .map(|typ| typ.as_str_name()),
        "type_name": def.type_name,
        "size": def.size,
        "precision": def.precision,
        "scale": def.scale,
    })
}

#[cfg(test)]
#[path = "spec_json_tests.rs"]
mod tests;
