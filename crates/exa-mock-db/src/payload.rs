//! Input payload builder: column classes to `column_definition`s and to
//! pre-encoded `MT_NEXT` reply frames, batched the way the database batches.
//!
//! Layout follows `exascript_table_data` in `zmqcontainer.proto`: one packed
//! array per cell type, cells in row-major order within each array, a
//! row-major `data_nulls` bitmap over every cell, and one `row_number` per row.

use chrono::{NaiveDate, NaiveDateTime};
use exa_proto::exascript_metadata::ColumnDefinition;
use exa_proto::{
    ColumnType, ExascriptMetadata, ExascriptNextDataRep, ExascriptResponse, ExascriptTableData,
    IterType, MessageType,
};
use prost::Message;

use crate::InputSource;
use crate::session::MOCK_CONN_ID;
use crate::sink::EMIT_LIMIT_BYTES;

/// Bytes the database counts for one int64 or double cell when filling a batch.
pub const FIXED_CELL_BYTES: usize = 8;

/// A source of input rows: their column metadata and the cells of row `i`.
pub trait RowSource {
    fn columns(&self) -> Vec<ColumnDefinition>;
    /// Append row `i` to every block of `table` (not `rows` or `row_number`)
    /// and return its byte cost: [`FIXED_CELL_BYTES`] per int64 or double
    /// cell, the byte length per string cell.
    fn write_row(&self, i: u64, table: &mut ExascriptTableData) -> usize;
}

/// The three benchmark column classes.
///
/// | class      | columns                                                       | on the wire                          |
/// |------------|---------------------------------------------------------------|--------------------------------------|
/// | `Native`   | `k DECIMAL(18,0), v DOUBLE`                                   | packed int64 and double arrays only  |
/// | `Strblock` | `k DECIMAL(18,0), amount DECIMAL(18,2), d DATE, ts TIMESTAMP` | three of four cells in `data_string` |
/// | `Varchar`  | `k DECIMAL(18,0), label VARCHAR(100)`                         | one 50-byte string per row           |
/// | `Wide`     | 24 columns of all three kinds, 12 nullable (`bench-udfs::WIDE_COLUMNS`) | emit-only: output metadata, no input rows |
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnClass {
    Native,
    Strblock,
    Varchar,
    Wide,
}

impl ColumnClass {
    /// Classes with source rows: input to RETURNS and SET cells.
    pub const ALL: [ColumnClass; 3] = [
        ColumnClass::Native,
        ColumnClass::Strblock,
        ColumnClass::Varchar,
    ];

    /// Classes a generator can emit.
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

pub use bench_schema::{WIDE_BATCH_ROWS, WIDE_COLUMNS};

/// Column definitions for [`WIDE_COLUMNS`].
pub fn wide_columns() -> Vec<ColumnDefinition> {
    WIDE_COLUMNS
        .iter()
        .map(|(name, ty, _)| match *ty {
            "DECIMAL(18,0)" => int64_col(name),
            "DOUBLE" => double_col(name),
            "BOOLEAN" => boolean_col(name),
            "DECIMAL(18,2)" => numeric_col(name, 18, 2),
            "DECIMAL(36,10)" => numeric_col(name, 36, 10),
            "DATE" => date_col(name),
            "TIMESTAMP" => timestamp_col(name),
            other => varchar_col(name, bench_schema::varchar_size(other).expect("VARCHAR(n)")),
        })
        .collect()
}

// --- column definitions ---------------------------------------------------

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

/// `DECIMAL(18,0)`, which the database delivers as a packed int64.
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

/// `DECIMAL(precision, scale)` in the string block.
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

pub fn date_col(name: &str) -> ColumnDefinition {
    column(name, ColumnType::PbDate, "DATE")
}

pub fn timestamp_col(name: &str) -> ColumnDefinition {
    column(name, ColumnType::PbTimestamp, "TIMESTAMP")
}

pub fn boolean_col(name: &str) -> ColumnDefinition {
    column(name, ColumnType::PbBoolean, "BOOLEAN")
}

pub fn varchar_col(name: &str, size: u32) -> ColumnDefinition {
    ColumnDefinition {
        size: Some(size),
        ..column(name, ColumnType::PbString, &format!("VARCHAR({size}) UTF8"))
    }
}

/// Handshake metadata for a data (non single-call) script.
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

// --- cell values, mirroring benches/bench-udfs -----------------------------

fn base_date() -> NaiveDate {
    NaiveDate::from_ymd_opt(2020, 1, 1).expect("valid base date")
}

/// `amount` for row `i` rendered as the database renders a `DECIMAL(18,2)`.
pub fn amount_text(i: u64) -> String {
    let unscaled = i as i128 * 137 + 4200;
    format!("{}.{:02}", unscaled / 100, unscaled % 100)
}

pub fn date_value(i: u64) -> NaiveDate {
    base_date() + chrono::Duration::days((i % 3650) as i64)
}

/// `ts` for row `i` at microsecond granularity, the resolution the database
/// delivers TIMESTAMP inputs at.
pub fn timestamp_value(i: u64) -> NaiveDateTime {
    let micros = (i * 137 % 1_000_000) as i64;
    base_date().and_hms_opt(0, 0, 0).expect("midnight")
        + chrono::Duration::seconds(i as i64)
        + chrono::Duration::microseconds(micros)
}

pub fn date_text(i: u64) -> String {
    date_value(i).format("%Y-%m-%d").to_string()
}

pub fn timestamp_text(i: u64) -> String {
    timestamp_value(i)
        .format("%Y-%m-%d %H:%M:%S.%6f")
        .to_string()
}

/// Fifty-byte label for row `i`.
pub fn label_text(i: u64) -> String {
    format!("{i:0>50}")
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

impl RowSource for ColumnClass {
    fn columns(&self) -> Vec<ColumnDefinition> {
        match self {
            ColumnClass::Native => vec![int64_col("k"), double_col("v")],
            ColumnClass::Strblock => vec![
                int64_col("k"),
                numeric_col("amount", 18, 2),
                date_col("d"),
                timestamp_col("ts"),
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
            ColumnClass::Varchar => push_int64(table, k) + push_string(table, label_text(i)),
            ColumnClass::Wide => panic!("ColumnClass::Wide is emit-only and has no input rows"),
        }
    }
}

/// Rows of `DECIMAL(18,0)` columns with caller-supplied values, for parameter
/// rows such as a generator's `(n, do_emit)` or a pass-through's `(k, n)`.
pub struct Int64Columns {
    pub names: Vec<String>,
    pub fill: Box<dyn Fn(u64) -> Vec<i64>>,
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

// --- encoding ---------------------------------------------------------------

/// Pre-encoded input for one benchmark cell: `cycles[c]` holds the `MT_NEXT`
/// reply frames of run cycle `c`.
pub struct EncodedInput {
    pub cycles: Vec<Vec<Vec<u8>>>,
    /// Total rows across all cycles.
    pub rows: u64,
}

impl EncodedInput {
    /// Frames across all cycles.
    pub fn frames(&self) -> usize {
        self.cycles.iter().map(Vec::len).sum()
    }

    /// Bytes across all frames.
    pub fn bytes(&self) -> usize {
        self.cycles.iter().flatten().map(Vec::len).sum()
    }
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

/// Encode rows `[first, last)` as the frames of one cycle, batched as the
/// database batches: cells accumulate until the running byte count passes
/// [`EMIT_LIMIT_BYTES`] or the rows run out. `row_number` is stamped from
/// `next_row_number`, which advances by one per row.
pub fn encode_cycle(
    src: &dyn RowSource,
    first: u64,
    last: u64,
    next_row_number: &mut u64,
) -> Vec<Vec<u8>> {
    let mut frames = Vec::new();
    let mut table = ExascriptTableData::default();
    let mut bytes = 0usize;
    for i in first..last {
        bytes += src.write_row(i, &mut table);
        table.rows += 1;
        table.row_number.push(*next_row_number);
        *next_row_number += 1;
        if bytes > EMIT_LIMIT_BYTES {
            frames.push(encode_frame(std::mem::take(&mut table)));
            bytes = 0;
        }
    }
    if table.rows > 0 {
        frames.push(encode_frame(table));
    }
    frames
}

/// Encode `rows` rows of `src` spread evenly over `cycles` run cycles, in
/// row order, with a single monotonic `row_number` sequence across the whole
/// input. Cycles that receive no rows are still present (and empty), so the
/// client sees `MT_DONE` at once for them.
pub fn encode(src: &dyn RowSource, rows: u64, cycles: u64) -> EncodedInput {
    assert!(cycles > 0, "at least one cycle");
    let mut next_row_number = 0u64;
    let mut out = Vec::with_capacity(cycles as usize);
    for c in 0..cycles {
        let first = rows * c / cycles;
        let last = rows * (c + 1) / cycles;
        out.push(encode_cycle(src, first, last, &mut next_row_number));
    }
    EncodedInput { cycles: out, rows }
}

/// Cursor over one cycle's frames.
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

/// Decode a frame produced by [`encode_cycle`] back into its table.
pub fn decode_frame(frame: &[u8]) -> ExascriptTableData {
    ExascriptResponse::decode(frame)
        .expect("frame is an exascript_response")
        .next
        .expect("MT_NEXT frame carries a table")
        .table
}

#[cfg(test)]
#[path = "payload_tests.rs"]
mod tests;
