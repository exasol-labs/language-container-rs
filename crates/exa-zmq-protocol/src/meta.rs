use crate::error::ProtocolError;
use exa_proto::{ColumnType, ExascriptInfo, ExascriptMetadata, IterType as PbIterType};

#[derive(Debug, Clone, PartialEq)]
pub enum ExaType {
    Unsupported,
    Double,
    Int32,
    Int64,
    Numeric,
    Timestamp,
    Date,
    String,
    Boolean,
}

#[derive(Debug, Clone, PartialEq)]
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
    pub input_iter: IterType,
    pub output_iter: IterType,
    pub input_columns: Vec<ColumnMeta>,
    pub output_columns: Vec<ColumnMeta>,
    pub single_call_mode: bool,
    pub source_code: String,
    pub script_name: String,
    pub session_id: u64,
    pub node_id: u32,
    pub node_count: u32,
    /// Connect-back credentials surfaced during the handshake, when the DB
    /// provided them (via an `MT_IMPORT` connection-information exchange).
    pub conn_info: Option<ConnInfo>,
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
            ColumnType::PbNumeric => ExaType::Numeric,
            ColumnType::PbTimestamp => ExaType::Timestamp,
            ColumnType::PbDate => ExaType::Date,
            ColumnType::PbString => ExaType::String,
            ColumnType::PbBoolean => ExaType::Boolean,
            ColumnType::PbUnsupported => ExaType::Unsupported,
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

    pub fn to_pb(&self) -> exa_proto::exascript_metadata::ColumnDefinition {
        let pb_type = match self.typ {
            ExaType::Double => ColumnType::PbDouble,
            ExaType::Int32 => ColumnType::PbInt32,
            ExaType::Int64 => ColumnType::PbInt64,
            ExaType::Numeric => ColumnType::PbNumeric,
            ExaType::Timestamp => ColumnType::PbTimestamp,
            ExaType::Date => ColumnType::PbDate,
            ExaType::String => ColumnType::PbString,
            ExaType::Boolean => ColumnType::PbBoolean,
            ExaType::Unsupported => ColumnType::PbUnsupported,
        };
        exa_proto::exascript_metadata::ColumnDefinition {
            name: self.name.clone(),
            r#type: Some(pb_type as i32),
            type_name: self.type_name.clone(),
            size: self.size,
            precision: self.precision,
            scale: self.scale,
        }
    }
}

fn iter_from_pb(iter: PbIterType) -> IterType {
    match iter {
        PbIterType::PbExactlyOnce => IterType::ExactlyOnce,
        PbIterType::PbMultiple => IterType::Multiple,
    }
}

fn iter_to_pb(iter: &IterType) -> PbIterType {
    match iter {
        IterType::ExactlyOnce => PbIterType::PbExactlyOnce,
        IterType::Multiple => PbIterType::PbMultiple,
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
            session_id: info.session_id,
            node_id: info.node_id,
            node_count: info.node_count,
            conn_info: None,
        })
    }

    pub fn to_pb(&self) -> ExascriptMetadata {
        ExascriptMetadata {
            input_iter_type: iter_to_pb(&self.input_iter) as i32,
            output_iter_type: iter_to_pb(&self.output_iter) as i32,
            input_columns: self.input_columns.iter().map(ColumnMeta::to_pb).collect(),
            output_columns: self.output_columns.iter().map(ColumnMeta::to_pb).collect(),
            single_call_mode: self.single_call_mode,
        }
    }
}
