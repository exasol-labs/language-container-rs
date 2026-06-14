use crate::error::ProtocolError;
use exa_proto::{ColumnType, ExascriptInfo, ExascriptMetadata, IterType as PbIterType};
pub use exasol_udf_sdk::value::ExaType;

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

    pub fn to_pb(&self) -> exa_proto::exascript_metadata::ColumnDefinition {
        let pb_type = match self.typ {
            ExaType::Double => ColumnType::PbDouble,
            ExaType::Int32 => ColumnType::PbInt32,
            ExaType::Int64 => ColumnType::PbInt64,
            ExaType::Numeric { .. } => ColumnType::PbNumeric,
            ExaType::Timestamp | ExaType::TimestampTz => ColumnType::PbTimestamp,
            ExaType::Date => ColumnType::PbDate,
            ExaType::String { .. }
            | ExaType::Char { .. }
            | ExaType::Geometry
            | ExaType::HashType
            | ExaType::IntervalYearToMonth
            | ExaType::IntervalDayToSecond => ColumnType::PbString,
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

#[cfg(test)]
mod tests {
    use super::*;
    use exa_proto::exascript_metadata::ColumnDefinition;

    fn col(
        ty: ColumnType,
        type_name: &str,
        size: Option<u32>,
        precision: Option<u32>,
        scale: Option<u32>,
    ) -> ColumnDefinition {
        ColumnDefinition {
            name: "c".to_string(),
            r#type: Some(ty as i32),
            type_name: type_name.to_string(),
            size,
            precision,
            scale,
        }
    }

    #[test]
    fn from_pb_uses_sdk_exatype() {
        let pb = col(ColumnType::PbNumeric, "DECIMAL", None, Some(18), Some(2));
        let meta = ColumnMeta::from_pb(&pb);
        assert_eq!(
            meta.typ,
            ExaType::Numeric {
                precision: Some(18),
                scale: Some(2)
            }
        );
    }

    #[test]
    fn from_pb_refines_extended_types_via_type_name() {
        let cases = [
            (
                "CHAR(10) UTF8",
                ColumnType::PbString,
                Some(10),
                ExaType::Char { size: Some(10) },
            ),
            (
                "VARCHAR(256) UTF8",
                ColumnType::PbString,
                Some(256),
                ExaType::String { size: Some(256) },
            ),
            ("GEOMETRY(0)", ColumnType::PbString, None, ExaType::Geometry),
            (
                "HASHTYPE(16 BYTE)",
                ColumnType::PbString,
                None,
                ExaType::HashType,
            ),
            (
                "INTERVAL YEAR(2) TO MONTH",
                ColumnType::PbString,
                None,
                ExaType::IntervalYearToMonth,
            ),
            (
                "INTERVAL DAY(2) TO SECOND(3)",
                ColumnType::PbString,
                None,
                ExaType::IntervalDayToSecond,
            ),
            (
                "TIMESTAMP(3) WITH LOCAL TIME ZONE",
                ColumnType::PbTimestamp,
                None,
                ExaType::TimestampTz,
            ),
        ];

        for (type_name, pb_ty, size, expected) in cases {
            let pb = col(pb_ty, type_name, size, None, None);
            let meta = ColumnMeta::from_pb(&pb);
            assert_eq!(meta.typ, expected, "type_name = {type_name}");
        }

        let plain_ts = col(ColumnType::PbTimestamp, "TIMESTAMP(3)", None, None, None);
        assert_eq!(ColumnMeta::from_pb(&plain_ts).typ, ExaType::Timestamp);

        let unknown_string = col(ColumnType::PbString, "MYSTERY", Some(7), None, None);
        assert_eq!(
            ColumnMeta::from_pb(&unknown_string).typ,
            ExaType::String { size: Some(7) }
        );
    }

    #[test]
    fn unambiguous_types_ignore_type_name() {
        let misleading = "VARCHAR WITH LOCAL TIME ZONE GEOMETRY";
        let cases = [
            (ColumnType::PbDouble, ExaType::Double),
            (ColumnType::PbInt32, ExaType::Int32),
            (ColumnType::PbInt64, ExaType::Int64),
            (ColumnType::PbDate, ExaType::Date),
            (ColumnType::PbBoolean, ExaType::Boolean),
            (ColumnType::PbUnsupported, ExaType::Unsupported),
        ];
        for (pb_ty, expected) in cases {
            let pb = col(pb_ty, misleading, None, None, None);
            assert_eq!(ColumnMeta::from_pb(&pb).typ, expected);
        }

        let numeric = col(ColumnType::PbNumeric, misleading, None, Some(5), Some(1));
        assert_eq!(
            ColumnMeta::from_pb(&numeric).typ,
            ExaType::Numeric {
                precision: Some(5),
                scale: Some(1)
            }
        );
    }

    #[test]
    fn extended_exatype_roundtrips_to_pb() {
        let cases = [
            (
                ExaType::Char { size: Some(10) },
                "CHAR(10)",
                ColumnType::PbString,
            ),
            (ExaType::Geometry, "GEOMETRY(0)", ColumnType::PbString),
            (ExaType::HashType, "HASHTYPE(16 BYTE)", ColumnType::PbString),
            (
                ExaType::IntervalYearToMonth,
                "INTERVAL YEAR TO MONTH",
                ColumnType::PbString,
            ),
            (
                ExaType::IntervalDayToSecond,
                "INTERVAL DAY TO SECOND",
                ColumnType::PbString,
            ),
            (
                ExaType::TimestampTz,
                "TIMESTAMP WITH LOCAL TIME ZONE",
                ColumnType::PbTimestamp,
            ),
        ];

        for (typ, type_name, expected_pb) in cases {
            let meta = ColumnMeta {
                name: "c".to_string(),
                typ: typ.clone(),
                type_name: type_name.to_string(),
                size: Some(10),
                precision: Some(3),
                scale: Some(1),
            };
            let pb = meta.to_pb();
            assert_eq!(pb.r#type(), expected_pb, "typ = {typ:?}");
            assert_eq!(pb.type_name, type_name);
            assert_eq!(pb.size, Some(10));
            assert_eq!(pb.precision, Some(3));
            assert_eq!(pb.scale, Some(1));
        }
    }
}
