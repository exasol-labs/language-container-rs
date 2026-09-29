//! Column classes, handshake metadata and `MT_NEXT` frames batched the way the
//! database batches: cells accumulate until the byte count passes the limit.

use chrono::NaiveDate;
use exa_proto::exascript_metadata::ColumnDefinition;
use exa_proto::{
    ColumnType, ExascriptMetadata, ExascriptNextDataRep, ExascriptResponse, ExascriptTableData,
    IterType, MessageType,
};
use prost::Message;

use crate::InputSource;
use crate::session::MOCK_CONN_ID;
use crate::sink::EMIT_LIMIT_BYTES;

pub use bench_schema::{WIDE_BATCH_ROWS, WIDE_COLUMNS};

const FIXED_CELL_BYTES: usize = 8;

pub trait RowSource {
    fn columns(&self) -> Vec<ColumnDefinition>;
    /// Appends row `i` to the data blocks and returns its byte cost.
    fn write_row(&self, i: u64, table: &mut ExascriptTableData) -> usize;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnClass {
    Native,
    Strblock,
    Varchar,
    /// 24 mixed-type columns, 12 nullable: generator output and `set_returns` input.
    Wide,
}

impl ColumnClass {
    pub const ALL: [ColumnClass; 3] = [
        ColumnClass::Native,
        ColumnClass::Strblock,
        ColumnClass::Varchar,
    ];
    pub const GEN: [ColumnClass; 4] = [
        ColumnClass::Native,
        ColumnClass::Strblock,
        ColumnClass::Varchar,
        ColumnClass::Wide,
    ];

    pub fn name(self) -> &'static str {
        match self {
            ColumnClass::Native => "native",
            ColumnClass::Strblock => "strblock",
            ColumnClass::Varchar => "varchar",
            ColumnClass::Wide => "wide",
        }
    }
}

fn column(name: &str, ty: ColumnType, type_name: &str) -> ColumnDefinition {
    ColumnDefinition {
        name: name.into(),
        r#type: Some(ty as i32),
        type_name: type_name.into(),
        size: None,
        precision: None,
        scale: None,
    }
}

pub fn int64_col(name: &str) -> ColumnDefinition {
    ColumnDefinition {
        precision: Some(18),
        scale: Some(0),
        ..column(name, ColumnType::PbInt64, "DECIMAL(18,0)")
    }
}

pub fn double_col(name: &str) -> ColumnDefinition {
    column(name, ColumnType::PbDouble, "DOUBLE")
}

pub fn numeric_col(name: &str, precision: u32, scale: u32) -> ColumnDefinition {
    ColumnDefinition {
        precision: Some(precision),
        scale: Some(scale),
        ..column(
            name,
            ColumnType::PbNumeric,
            &format!("DECIMAL({precision},{scale})"),
        )
    }
}

fn varchar_col(name: &str, size: u32) -> ColumnDefinition {
    ColumnDefinition {
        size: Some(size),
        ..column(name, ColumnType::PbString, &format!("VARCHAR({size}) UTF8"))
    }
}

fn wide_columns() -> Vec<ColumnDefinition> {
    WIDE_COLUMNS
        .iter()
        .map(|(name, ty, _)| match *ty {
            "DECIMAL(18,0)" => int64_col(name),
            "DOUBLE" => double_col(name),
            "BOOLEAN" => column(name, ColumnType::PbBoolean, "BOOLEAN"),
            "DECIMAL(18,2)" => numeric_col(name, 18, 2),
            "DECIMAL(36,10)" => numeric_col(name, 36, 10),
            "DATE" => column(name, ColumnType::PbDate, "DATE"),
            "TIMESTAMP" => column(name, ColumnType::PbTimestamp, "TIMESTAMP"),
            other => varchar_col(name, bench_schema::varchar_size(other).expect("VARCHAR(n)")),
        })
        .collect()
}

pub fn metadata(
    input_iter: IterType,
    output_iter: IterType,
    input_columns: Vec<ColumnDefinition>,
    output_columns: Vec<ColumnDefinition>,
) -> ExascriptMetadata {
    ExascriptMetadata {
        input_iter_type: input_iter as i32,
        output_iter_type: output_iter as i32,
        input_columns,
        output_columns,
        single_call_mode: false,
    }
}

fn base() -> NaiveDate {
    NaiveDate::from_ymd_opt(2020, 1, 1).unwrap()
}

fn amount_text(i: u64) -> String {
    let unscaled = i as i128 * 137 + 4200;
    format!("{}.{:02}", unscaled / 100, unscaled % 100)
}

fn date_text(i: u64) -> String {
    (base() + chrono::Duration::days((i % 3650) as i64))
        .format("%Y-%m-%d")
        .to_string()
}

/// Microsecond granularity, the resolution the database delivers TIMESTAMP inputs at.
fn timestamp_text(i: u64) -> String {
    (base().and_hms_opt(0, 0, 0).unwrap()
        + chrono::Duration::seconds(i as i64)
        + chrono::Duration::microseconds((i * 137 % 1_000_000) as i64))
    .format("%Y-%m-%d %H:%M:%S.%6f")
    .to_string()
}

fn push_int64(table: &mut ExascriptTableData, v: i64) -> usize {
    table.data_int64.push(v);
    table.data_nulls.push(false);
    FIXED_CELL_BYTES
}

fn push_double(table: &mut ExascriptTableData, v: f64) -> usize {
    table.data_double.push(v);
    table.data_nulls.push(false);
    FIXED_CELL_BYTES
}

fn push_string(table: &mut ExascriptTableData, s: String) -> usize {
    let len = s.len();
    table.data_string.push(s);
    table.data_nulls.push(false);
    len
}

fn push_bool(table: &mut ExascriptTableData, v: bool) -> usize {
    table.data_bool.push(v);
    table.data_nulls.push(false);
    1
}

fn push_null(table: &mut ExascriptTableData) -> usize {
    table.data_nulls.push(true);
    0
}

fn big_text(i: u64) -> String {
    let unscaled = i as i128 * 12_345_678_901 + 987_654_321;
    format!(
        "{}.{:010}",
        unscaled / 10_000_000_000,
        unscaled % 10_000_000_000
    )
}

const WIDE_TEXT: &str = "the quick brown fox jumps over the lazy dog while forty-two \
    benchmark rows stream through a script language container, each carrying a \
    reference to a file whose contents are emitted as wide rows with dozens of \
    columns of mixed types and a sprinkling of nulls";

fn wide_text(i: u64, col: usize) -> String {
    let max = bench_schema::varchar_size(WIDE_COLUMNS[col].1).expect("VARCHAR(n)") as usize;
    let len = 1 + (i as usize * 31 + col * 7) % max;
    let start = (i as usize * 13 + col) % (WIDE_TEXT.len() - len);
    WIDE_TEXT[start..start + len].to_string()
}

/// Same NULL pattern as the bench UDF's wide generator: every tenth cell of a
/// nullable column, staggered by column.
fn wide_is_null(i: u64, col: usize) -> bool {
    WIDE_COLUMNS[col].2 && (i + col as u64).is_multiple_of(10)
}

fn write_wide_row(i: u64, t: &mut ExascriptTableData) -> usize {
    let k = i as i64;
    (0..WIDE_COLUMNS.len())
        .map(|col| {
            if wide_is_null(i, col) {
                return push_null(t);
            }
            match col {
                0 => push_int64(t, k),
                1 => push_int64(t, k * 7),
                2 => push_int64(t, k % 1_000),
                3 => push_double(t, k as f64 * 1.5),
                4 => push_double(t, k as f64 / 3.0),
                5 => push_double(t, -(k as f64) * 0.25),
                6 => push_bool(t, i.is_multiple_of(2)),
                7 => push_bool(t, i.is_multiple_of(3)),
                8 => push_string(t, amount_text(i)),
                9 => push_string(t, amount_text(i + 1)),
                10 => push_string(t, big_text(i)),
                11 => push_string(t, big_text(i + 1)),
                12 => push_string(t, date_text(i)),
                13 => push_string(t, date_text(i + 1)),
                14 => push_string(t, timestamp_text(i)),
                15 => push_string(t, timestamp_text(i + 1)),
                _ => push_string(t, wide_text(i, col)),
            }
        })
        .sum()
}

impl RowSource for ColumnClass {
    fn columns(&self) -> Vec<ColumnDefinition> {
        match self {
            ColumnClass::Native => vec![int64_col("k"), double_col("v")],
            ColumnClass::Strblock => vec![
                int64_col("k"),
                numeric_col("amount", 18, 2),
                column("d", ColumnType::PbDate, "DATE"),
                column("ts", ColumnType::PbTimestamp, "TIMESTAMP"),
            ],
            ColumnClass::Varchar => vec![int64_col("k"), varchar_col("label", 100)],
            ColumnClass::Wide => wide_columns(),
        }
    }

    fn write_row(&self, i: u64, table: &mut ExascriptTableData) -> usize {
        let k = i as i64;
        match self {
            ColumnClass::Native => push_int64(table, k) + push_double(table, k as f64 * 1.5),
            ColumnClass::Strblock => {
                push_int64(table, k)
                    + push_string(table, amount_text(i))
                    + push_string(table, date_text(i))
                    + push_string(table, timestamp_text(i))
            }
            ColumnClass::Varchar => push_int64(table, k) + push_string(table, format!("{i:0>50}")),
            ColumnClass::Wide => write_wide_row(i, table),
        }
    }
}

pub struct Int64Columns {
    names: Vec<String>,
    fill: Box<dyn Fn(u64) -> Vec<i64>>,
}

impl Int64Columns {
    pub fn new(names: &[&str], fill: impl Fn(u64) -> Vec<i64> + 'static) -> Self {
        Int64Columns {
            names: names.iter().map(|s| s.to_string()).collect(),
            fill: Box::new(fill),
        }
    }
}

impl RowSource for Int64Columns {
    fn columns(&self) -> Vec<ColumnDefinition> {
        self.names.iter().map(|n| int64_col(n)).collect()
    }

    fn write_row(&self, i: u64, table: &mut ExascriptTableData) -> usize {
        let values = (self.fill)(i);
        assert_eq!(values.len(), self.names.len(), "Int64Columns fill arity");
        values.into_iter().map(|v| push_int64(table, v)).sum()
    }
}

/// `cycles[c]` holds the `MT_NEXT` frames of run cycle `c`.
pub struct EncodedInput {
    pub cycles: Vec<Vec<Vec<u8>>>,
}

fn encode_frame(table: ExascriptTableData) -> Vec<u8> {
    ExascriptResponse {
        r#type: MessageType::MtNext as i32,
        connection_id: MOCK_CONN_ID,
        next: Some(ExascriptNextDataRep { table }),
        ..Default::default()
    }
    .encode_to_vec()
}

/// Spreads `rows` evenly over `cycles` (empty cycles stay present), with one
/// monotonic `row_number` sequence across the whole input.
pub fn encode(src: &dyn RowSource, rows: u64, cycles: u64) -> EncodedInput {
    assert!(cycles > 0);
    let mut row_number = 0u64;
    let out = (0..cycles)
        .map(|c| {
            let mut frames = Vec::new();
            let mut table = ExascriptTableData::default();
            let mut bytes = 0usize;
            for i in rows * c / cycles..rows * (c + 1) / cycles {
                bytes += src.write_row(i, &mut table);
                table.rows += 1;
                table.row_number.push(row_number);
                row_number += 1;
                if bytes > EMIT_LIMIT_BYTES {
                    frames.push(encode_frame(std::mem::take(&mut table)));
                    bytes = 0;
                }
            }
            if table.rows > 0 {
                frames.push(encode_frame(table));
            }
            frames
        })
        .collect();
    EncodedInput { cycles: out }
}

pub struct FrameCursor<'a> {
    frames: &'a [Vec<u8>],
    pos: usize,
}

impl<'a> FrameCursor<'a> {
    pub fn new(frames: &'a [Vec<u8>]) -> Self {
        FrameCursor { frames, pos: 0 }
    }
}

impl InputSource for FrameCursor<'_> {
    fn next_frame(&mut self) -> Option<&[u8]> {
        let f = self.frames.get(self.pos)?;
        self.pos += 1;
        Some(f.as_slice())
    }
}

#[cfg(test)]
#[path = "payload_tests.rs"]
mod tests;
