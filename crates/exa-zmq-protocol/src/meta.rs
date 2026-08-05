use crate::error::ProtocolError;
use exa_proto::{ColumnType, ExascriptInfo, ExascriptMetadata, IterType as PbIterType};
pub use exasol_udf_sdk::value::ExaType;

/// Iteration axis for a UDF's input or output: `ExactlyOnce` is the scalar /
/// RETURNS shape (one row per invocation), `Multiple` the set / EMITS shape
/// (many rows per invocation). Parsed from the handshake metadata; the run
/// dispatcher branches on the input axis to drive the UDF per-row or per-group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IterType {
    ExactlyOnce,
    Multiple,
}

#[derive(Debug, Clone)]
pub struct ColumnMeta {
    pub name: String,
    pub typ: ExaType,
    pub type_name: String,
    pub size: Option<u32>,
    pub precision: Option<u32>,
    pub scale: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct UdfMeta {
    pub(crate) input_iter: IterType,
    pub(crate) output_iter: IterType,
    pub input_columns: Vec<ColumnMeta>,
    pub output_columns: Vec<ColumnMeta>,
    pub single_call_mode: bool,
    pub source_code: String,
    pub script_name: String,
    pub script_schema: String,
    pub database_name: String,
    pub database_version: String,
    pub(crate) session_id: u64,
    pub(crate) statement_id: u32,
    pub(crate) node_id: u32,
    pub(crate) node_count: u32,
    pub(crate) vm_id: u64,
    /// Current user reported by the DB, when present (proto `optional`).
    pub current_user: Option<String>,
    /// Current schema reported by the DB, when present (proto `optional`).
    pub current_schema: Option<String>,
    /// Scope user reported by the DB, when present (proto `optional`).
    pub scope_user: Option<String>,
    /// Bytes, per-UDF-instance resident-memory limit.
    pub maximal_memory_limit: u64,
}

/// Connection credentials returned by the DB in response to an MT_IMPORT
/// request with `PB_IMPORT_CONNECTION_INFORMATION`.
#[derive(Debug, Clone)]
pub struct ConnInfo {
    pub kind: String,
    pub address: String,
    pub user: String,
    pub password: String,
}

impl ConnInfo {
    pub(crate) fn from_pb(pb: exa_proto::ConnectionInformationRep) -> Self {
        ConnInfo {
            kind: pb.kind,
            address: pb.address,
            user: pb.user,
            password: pb.password,
        }
    }
}

impl ColumnMeta {
    pub fn from_pb(col: &exa_proto::exascript_metadata::ColumnDefinition) -> Self {
        let typ = match col.r#type() {
            ColumnType::PbDouble => ExaType::Double,
            ColumnType::PbInt32 => ExaType::Int32,
            ColumnType::PbInt64 => ExaType::Int64,
            ColumnType::PbNumeric => ExaType::Numeric {
                precision: col.precision,
                scale: col.scale,
            },
            ColumnType::PbDate => ExaType::Date,
            ColumnType::PbBoolean => ExaType::Boolean,
            ColumnType::PbUnsupported => ExaType::Unsupported,
            ColumnType::PbTimestamp => refine_timestamp(&col.type_name),
            ColumnType::PbString => refine_string(&col.type_name, col.size),
        };
        ColumnMeta {
            name: col.name.clone(),
            typ,
            type_name: col.type_name.clone(),
            size: col.size,
            precision: col.precision,
            scale: col.scale,
        }
    }
}

fn refine_string(type_name: &str, size: Option<u32>) -> ExaType {
    if type_name.starts_with("CHAR") {
        ExaType::Char { size }
    } else if type_name.starts_with("VARCHAR") {
        ExaType::String { size }
    } else if type_name.starts_with("GEOMETRY") {
        ExaType::Geometry
    } else if type_name.starts_with("HASHTYPE") {
        ExaType::HashType
    } else if type_name.contains("YEAR") && type_name.contains("MONTH") {
        ExaType::IntervalYearToMonth
    } else if type_name.contains("DAY") && type_name.contains("SECOND") {
        ExaType::IntervalDayToSecond
    } else {
        ExaType::String { size }
    }
}

fn refine_timestamp(type_name: &str) -> ExaType {
    if type_name.contains("LOCAL TIME ZONE") {
        ExaType::TimestampTz
    } else {
        ExaType::Timestamp
    }
}

fn iter_from_pb(iter: PbIterType) -> IterType {
    match iter {
        PbIterType::PbExactlyOnce => IterType::ExactlyOnce,
        PbIterType::PbMultiple => IterType::Multiple,
    }
}

impl UdfMeta {
    pub fn from_pb(meta: &ExascriptMetadata, info: &ExascriptInfo) -> Result<Self, ProtocolError> {
        Ok(UdfMeta {
            input_iter: iter_from_pb(meta.input_iter_type()),
            output_iter: iter_from_pb(meta.output_iter_type()),
            input_columns: meta.input_columns.iter().map(ColumnMeta::from_pb).collect(),
            output_columns: meta
                .output_columns
                .iter()
                .map(ColumnMeta::from_pb)
                .collect(),
            single_call_mode: meta.single_call_mode,
            source_code: info.source_code.clone(),
            script_name: info.script_name.clone(),
            script_schema: info.script_schema.clone(),
            database_name: info.database_name.clone(),
            database_version: info.database_version.clone(),
            session_id: info.session_id,
            statement_id: info.statement_id,
            node_id: info.node_id,
            node_count: info.node_count,
            vm_id: info.vm_id,
            current_user: info.current_user.clone(),
            current_schema: info.current_schema.clone(),
            scope_user: info.scope_user.clone(),
            maximal_memory_limit: info.maximal_memory_limit,
        })
    }

    /// Session ID of the current Exasol session.
    pub fn session_id(&self) -> u64 {
        self.session_id
    }

    /// Statement number within the current session.
    pub fn statement_id(&self) -> u32 {
        self.statement_id
    }

    /// Node ID (0-based) of the cluster node running this UDF instance.
    pub fn node_id(&self) -> u32 {
        self.node_id
    }

    /// Number of nodes in the Exasol cluster.
    pub fn node_count(&self) -> u32 {
        self.node_count
    }

    /// Long unique ID of the VM / UDF process instance.
    pub fn vm_id(&self) -> u64 {
        self.vm_id
    }

    /// Input iteration axis: `ExactlyOnce` for SCALAR (per-row dispatch),
    /// `Multiple` for SET (per-group dispatch).
    pub fn input_iter(&self) -> IterType {
        self.input_iter
    }

    /// Output iteration axis: `ExactlyOnce` for RETURNS, `Multiple` for EMITS.
    pub fn output_iter(&self) -> IterType {
        self.output_iter
    }
}

#[cfg(test)]
#[path = "meta_tests.rs"]
mod tests;
