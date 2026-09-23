//! Typed views of the specification payload a spec-generation hook receives.
//!
//! The hook's parameter is the raw JSON string in every build configuration, so
//! the vtable layout never depends on a cargo feature. Parsing it is the step an
//! author opts into: the `import` feature ships [`ImportSpec`], the `export`
//! feature ships [`ExportSpec`], and a UDF that writes one kind of hook carries
//! no code for the other.
//!
//! Each struct mirrors its protobuf message field for field under the proto
//! field names and normalizes nothing — `type` stays the proto variant name
//! such as `"PB_DOUBLE"` — so the proto-to-[`crate::ExaType`] mapping keeps its
//! single owner in `exa-zmq-protocol`. Unknown JSON fields are ignored, so a
//! `.so` built against this SDK keeps parsing a payload that a later proto
//! field widened.

use serde::Deserialize;

use crate::connect_back::ConnectionObject;
use crate::error::UdfError;

/// One entry of the statement's `WITH` clause, mirroring the proto
/// `key_value_pair`. Parsed as an ordered list rather than a map because the
/// map form silently drops a repeated key.
#[derive(Debug, Clone, Deserialize)]
pub struct Parameter {
    pub key: String,
    pub value: String,
}

/// A column of the `IMPORT INTO (...)` list, mirroring the proto
/// `exascript_metadata.column_definition`.
#[cfg(feature = "import")]
#[derive(Debug, Clone, Deserialize)]
pub struct ColumnDefinition {
    pub name: String,
    /// The proto `column_type` variant name, for example `"PB_DOUBLE"`, or
    /// `None` when the database did not report one.
    pub r#type: Option<String>,
    /// The type as Exasol displays it, for example `"VARCHAR(20) UTF8"`.
    pub type_name: String,
    pub size: Option<u32>,
    pub precision: Option<u32>,
    pub scale: Option<u32>,
}

/// The `IMPORT ... FROM SCRIPT` specification, mirroring the proto
/// `import_specification_rep`.
#[cfg(feature = "import")]
#[derive(Debug, Clone, Deserialize)]
pub struct ImportSpec {
    pub is_subselect: bool,
    pub connection_information: Option<ConnectionObject>,
    pub connection_name: Option<String>,
    pub subselect_column_specification: Vec<ColumnDefinition>,
    pub parameters: Vec<Parameter>,
}

#[cfg(feature = "import")]
impl ImportSpec {
    /// Parse the `json_spec` a `generate_sql_for_import_spec` hook receives.
    pub fn from_json(json_spec: &str) -> Result<Self, UdfError> {
        serde_json::from_str(json_spec)
            .map_err(|e| UdfError::Type(format!("parsing the IMPORT specification: {e}")))
    }
}

/// The `EXPORT ... INTO SCRIPT` specification, mirroring the proto
/// `export_specification_rep`.
#[cfg(feature = "export")]
#[derive(Debug, Clone, Deserialize)]
pub struct ExportSpec {
    pub has_truncate: bool,
    pub has_replace: bool,
    pub created_by: Option<String>,
    pub source_column_names: Vec<String>,
    pub connection_information: Option<ConnectionObject>,
    pub connection_name: Option<String>,
    pub parameters: Vec<Parameter>,
}

#[cfg(feature = "export")]
impl ExportSpec {
    /// Parse the `json_spec` a `generate_sql_for_export_spec` hook receives.
    pub fn from_json(json_spec: &str) -> Result<Self, UdfError> {
        serde_json::from_str(json_spec)
            .map_err(|e| UdfError::Type(format!("parsing the EXPORT specification: {e}")))
    }
}

#[cfg(all(test, feature = "import", feature = "export"))]
#[path = "spec_tests.rs"]
mod tests;
