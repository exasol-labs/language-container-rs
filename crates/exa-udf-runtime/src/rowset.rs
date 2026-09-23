use chrono::{NaiveDate, NaiveDateTime};
use exa_proto::ExascriptTableData;
use exa_zmq_protocol::{ColumnInfo, ExaType, IterType};
use exasol_udf_sdk::context::{InputType, OutputType, UdfContext};
use exasol_udf_sdk::error::UdfError;
use exasol_udf_sdk::value::{Decimal, Value};

/// Per-type column block width: each block holds exactly `n_rows` entries per
/// column (placeholder slots for NULL cells), so cell `(col, row)` lives at
/// `block_base + row`. The NULL bitmap is row-major: `row * n_cols + col`.
fn null_index(row: usize, col: usize, n_cols: usize) -> usize {
    row * n_cols + col
}

/// Maps the wire's iteration axis onto the SDK's input-axis vocabulary, shared
/// by `HostContextBridge` and `SingleCallContext` so both report the same
/// mapping from one declared `IterType`.
fn input_type_of(iter: IterType) -> InputType {
    match iter {
        IterType::ExactlyOnce => InputType::Scalar,
        IterType::Multiple => InputType::Set,
    }
}

/// Maps the wire's iteration axis onto the SDK's output-axis vocabulary, the
/// output-side counterpart of [`input_type_of`].
fn output_type_of(iter: IterType) -> OutputType {
    match iter {
        IterType::ExactlyOnce => OutputType::Returns,
        IterType::Multiple => OutputType::Emits,
    }
}

/// Materialised input rows from one proto `ExascriptTableData` batch.
///
/// Stored as a dense `rows[row][col]` matrix of `Value` for simplicity and
/// correctness; the per-type proto blocks are decoded once on construction.
/// `row_numbers` holds the DB's local row number per row; emitted rows echo it
/// so the engine can place them beside their input row. `rows_in_group` carries
/// the batch's own group row count through unchanged (a SET group size, or a
/// SCALAR vector-chunk size), so the host bridge answers `UdfContext::rows_in_group()`
/// from the current batch without reinterpreting the input shape.
pub struct InputRowSet {
    rows: Vec<Vec<Value>>,
    row_numbers: Vec<u64>,
    current_row: usize,
    rows_in_group: u64,
}

impl InputRowSet {
    /// Decode a proto batch into a row-major matrix of `Value`s.
    ///
    /// The proto packs each cell type into its own array, column by column,
    /// with one slot per row (including NULL cells). The NULL bitmap is
    /// row-major across all columns.
    pub fn from_proto(table: &ExascriptTableData, meta: &[ColumnInfo]) -> Self {
        let n_rows = table.rows as usize;
        let n_cols = meta.len();

        // Row-major within each type block: cells appear in (row, column) order,
        // so a column's value for row `r` is the `r`-th time that column's type
        // block is consumed while walking rows then columns. Per-type running
        // cursors advance only when a non-null cell of that type is read, mirroring
        // how `to_proto` packs (and how Exasol lays out emitted/input batches).
        let mut string_idx = 0usize;
        let mut bool_idx = 0usize;
        let mut int32_idx = 0usize;
        let mut int64_idx = 0usize;
        let mut double_idx = 0usize;

        let mut rows: Vec<Vec<Value>> = Vec::with_capacity(n_rows);
        for r in 0..n_rows {
            let mut row: Vec<Value> = Vec::with_capacity(n_cols);
            for (c, col) in meta.iter().enumerate() {
                let is_null = table
                    .data_nulls
                    .get(null_index(r, c, n_cols))
                    .copied()
                    .unwrap_or(false);
                if is_null {
                    // A NULL cell occupies no slot in its type block; do not
                    // advance the per-type cursor (see `to_proto`).
                    row.push(Value::Null);
                    continue;
                }
                let v = match &col.typ {
                    ExaType::Numeric { .. }
                    | ExaType::Date
                    | ExaType::Timestamp { .. }
                    | ExaType::String { .. }
                    | ExaType::Char { .. } => {
                        let s = table.data_string.get(string_idx).map_or("", String::as_str);
                        string_idx += 1;
                        decode_string_block(&col.typ, s)
                    }
                    ExaType::Boolean => {
                        let b = table.data_bool.get(bool_idx).copied().unwrap_or(false);
                        bool_idx += 1;
                        Value::Bool(b)
                    }
                    ExaType::Int32 => {
                        let i = table.data_int32.get(int32_idx).copied().unwrap_or(0);
                        int32_idx += 1;
                        Value::Int32(i)
                    }
                    ExaType::Int64 => {
                        let i = table.data_int64.get(int64_idx).copied().unwrap_or(0);
                        int64_idx += 1;
                        Value::Int64(i)
                    }
                    ExaType::Double => {
                        let f = table.data_double.get(double_idx).copied().unwrap_or(0.0);
                        double_idx += 1;
                        Value::Double(f)
                    }
                    ExaType::Unsupported => Value::Null,
                };
                row.push(v);
            }
            rows.push(row);
        }

        // A batch that carries no (or a short) `row_number` list falls back to
        // batch-local indices so the emit side always has a number to echo.
        let row_numbers = if table.row_number.len() >= n_rows {
            table.row_number[..n_rows].to_vec()
        } else {
            (0..n_rows as u64).collect()
        };

        InputRowSet {
            rows,
            row_numbers,
            current_row: 0,
            rows_in_group: table.rows_in_group,
        }
    }

    /// The group row count the database reported for the batch this row set
    /// was decoded from: the SET group size, or the SCALAR vector-chunk size.
    /// `0` reports a call with no group defined.
    pub fn rows_in_group(&self) -> u64 {
        self.rows_in_group
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Advance to the next row. Returns false when already on the last row.
    pub fn advance(&mut self) -> bool {
        if self.current_row + 1 < self.rows.len() {
            self.current_row += 1;
            true
        } else {
            false
        }
    }

    pub fn current_row(&self) -> &[Value] {
        &self.rows[self.current_row]
    }

    /// The DB's local row number for the row the cursor sits on.
    pub fn current_row_number(&self) -> u64 {
        self.row_numbers
            .get(self.current_row)
            .copied()
            .unwrap_or_default()
    }

    pub fn row(&self, idx: usize) -> Option<&[Value]> {
        self.rows.get(idx).map(|r| r.as_slice())
    }
}

/// Byte threshold at which the emit buffer requests a flush. Keeps the
/// serialised `MT_EMIT` payload well under the DB's per-message limits while
/// amortising the per-flush round-trip across many rows.
const EMIT_BUFFER_LIMIT_BYTES: usize = 4_000_000;

/// Fixed per-cell byte-cost widths shared by `value_byte_cost` (the `Value`
/// axis) and `fixed_cell_cost` (the Arrow `DataType` axis).
const BYTES_BOOL: usize = 1;
const BYTES_INT32: usize = 4;
const BYTES_INT64: usize = 8;
const BYTES_DOUBLE: usize = 8;
const BYTES_DATE: usize = 10;
const BYTES_TIMESTAMP: usize = 29;
/// Per-row cost of the `row_number` entry every emitted row carries (a packed
/// uint64 varint, at most 10 bytes).
const BYTES_ROW_NUMBER: usize = 10;
/// Fixed portion of a NUMERIC cell's cost; callers add `scale` digits on top.
/// O(1) upper bound, no alloc: i128 renders in ≤39 digits + sign, which also
/// dominates the scale-padded form. Over-counts → flushes early.
const NUMERIC_COST_BASE: usize = 40;

/// Conservative O(1) byte cost for one cell, approximating the width its
/// non-null value occupies in `to_proto`'s type block. NULL cells cost 0
/// because they take no type-block slot; the estimate slightly over-counts
/// (fixed widths for Numeric/Date/Timestamp) so the buffer flushes early rather
/// than late.
fn value_byte_cost(v: &Value) -> usize {
    match v {
        Value::Null => 0,
        Value::Bool(_) => BYTES_BOOL,
        Value::Int32(_) => BYTES_INT32,
        Value::Int64(_) => BYTES_INT64,
        Value::Double(_) => BYTES_DOUBLE,
        Value::String(s) => s.len(),
        Value::Numeric(d) => NUMERIC_COST_BASE + d.scale as usize,
        Value::Date(_) => BYTES_DATE,
        Value::Timestamp(_) => BYTES_TIMESTAMP,
    }
}

/// Read process RSS (resident set size) from `/proc/self/statm` field 2.
///
/// Returns kilobytes (pages × 4096 / 1024 = pages × 4).  Falls back to 0 on
/// any I/O or parse error so telemetry never panics.
///
/// `/proc/self/statm` format: `size resident shared text lib data dt`
/// Field 1 (0-indexed) is the resident page count.
/// Page size is 4096 on x86_64; hardcoded here to avoid a syscall on every
/// telemetry checkpoint — the 4 KiB page is universal on the Linux targets
/// this SLC runs on.
// ponytail: hardcoded 4096 page size; sysconf(SC_PAGESIZE) would be more
// correct but costs a syscall per checkpoint.
fn read_rss_kb() -> u64 {
    let Ok(contents) = std::fs::read_to_string("/proc/self/statm") else {
        return 0;
    };
    contents
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse::<u64>().ok())
        .map(|pages| pages * 4) // pages × 4096 bytes / 1024 = pages × 4 KiB
        .unwrap_or(0)
}

/// The session's resolved verbosity level, read from the process-global
/// `LevelFilter` that `Runtime::run`'s `on_level_resolved` hook adjusted after
/// parsing `%udf_debug_level`. Maps `OFF` (the pre-handshake default) to `INFO`
/// so UDF code gets a sensible default before the level is applied.
fn current_debug_level() -> tracing::Level {
    tracing::level_filters::LevelFilter::current()
        .into_level()
        .unwrap_or(tracing::Level::INFO)
}

/// Accumulates emitted output rows directly in the wire's proto type blocks, so
/// a flush is a `mem::take` and the row and Arrow batch paths share one buffer.
///
/// Each type block is dense (one slot per non-null cell) and filled row-major,
/// so columns sharing a block interleave; the NULL bitmap is row-major too.
/// `InputRowSet::from_proto` reads back exactly this layout.
#[derive(Default)]
pub struct EmitBuffer {
    strings: Vec<String>,
    nulls: Vec<bool>,
    bools: Vec<bool>,
    int32: Vec<i32>,
    int64: Vec<i64>,
    doubles: Vec<f64>,
    /// Local row number of the input row each buffered output row came from.
    row_numbers: Vec<u64>,
    rows: usize,
    /// Running approximate serialised size of the buffered rows.
    byte_estimate: usize,
    /// Total bytes emitted across all flushes (running sum, never reset).
    cumulative_bytes: usize,
    /// Total rows emitted across all flushes (running sum, never reset).
    cumulative_rows: u64,
    /// Number of `MT_EMIT` flushes performed.
    flush_count: u64,
}

impl EmitBuffer {
    pub fn new() -> Self {
        EmitBuffer::default()
    }

    /// Emit a periodic RSS + full-state checkpoint every this many cumulative rows.
    // ponytail: 10_000 rows per checkpoint; a long 60M-row UDF emits ~6000
    // checkpoint lines — noisy but bearable at debug level.
    const TELEMETRY_ROW_CHECKPOINT: u64 = 10_000;

    pub fn push(&mut self, values: Vec<Value>, row_number: u64, meta: &[ColumnInfo]) {
        let row_cost = BYTES_ROW_NUMBER + values.iter().map(value_byte_cost).sum::<usize>();
        self.push_costed(values, row_number, meta, row_cost);
    }

    /// `push` with the row's byte cost already computed. The bridge computes it
    /// in the pass that validates the row, so the row is not walked for it twice.
    pub fn push_costed(
        &mut self,
        mut values: Vec<Value>,
        row_number: u64,
        meta: &[ColumnInfo],
        row_cost: usize,
    ) {
        // Per-push debug event: bytes buffered and running cost for this row.
        // Automatic tracing-level gating suppresses this at INFO or coarser.
        tracing::debug!(
            target: "emit_push",
            bytes_buffered = self.byte_estimate + row_cost,
            row_cost,
            cumulative_rows = self.cumulative_rows + 1,
            "emit row buffered"
        );
        self.account(1, row_cost);
        for (c, col) in meta.iter().enumerate() {
            match values.get_mut(c).filter(|v| !matches!(v, Value::Null)) {
                // Exasol consumes type-block entries only for non-null cells, so
                // a placeholder here would shift every later cell of that type
                // into the wrong column.
                None => self.nulls.push(true),
                Some(v) => {
                    self.nulls.push(false);
                    self.push_cell(&col.typ, v);
                }
            }
        }
        self.row_numbers.push(row_number);
        self.rows += 1;
    }

    /// Pack one non-NULL cell into the block dictated by the declared column
    /// type, not the runtime `Value` variant: a connect-back SELECT may hand an
    /// `ExaType::Numeric` column a `Value::Int64`, which still goes to the
    /// string block.
    fn push_cell(&mut self, typ: &ExaType, v: &mut Value) {
        match typ {
            ExaType::Numeric { .. }
            | ExaType::Date
            | ExaType::Timestamp { .. }
            | ExaType::String { .. }
            | ExaType::Char { .. } => self.strings.push(value_take_block_string(v)),
            ExaType::Boolean => self.bools.push(value_to_bool(v)),
            ExaType::Int32 => self.int32.push(value_to_i64(v) as i32),
            ExaType::Int64 => self.int64.push(value_to_i64(v)),
            ExaType::Double => self.doubles.push(value_to_f64(v)),
            ExaType::Unsupported => {}
        }
    }

    /// Fold newly buffered rows into the counters, emitting an RSS checkpoint
    /// when the row total crosses a `TELEMETRY_ROW_CHECKPOINT` multiple.
    fn account(&mut self, rows: u64, cost: usize) {
        self.byte_estimate += cost;
        self.cumulative_bytes += cost;
        let before = self.cumulative_rows;
        self.cumulative_rows += rows;
        if before / Self::TELEMETRY_ROW_CHECKPOINT
            != self.cumulative_rows / Self::TELEMETRY_ROW_CHECKPOINT
        {
            self.record_flush_telemetry();
        }
    }

    /// Whether the buffered rows have reached the byte threshold and should be
    /// flushed to the DB. A single oversized row trips this on its own push.
    pub fn should_flush(&self) -> bool {
        self.byte_estimate >= EMIT_BUFFER_LIMIT_BYTES
    }

    /// Take the accumulated blocks as an `ExascriptTableData`, leaving the
    /// buffer empty and ready for the next batch.
    pub fn take_proto(&mut self) -> ExascriptTableData {
        let table = ExascriptTableData {
            rows: self.rows as u64,
            rows_in_group: 0,
            data_string: std::mem::take(&mut self.strings),
            data_nulls: std::mem::take(&mut self.nulls),
            data_bool: std::mem::take(&mut self.bools),
            data_int32: std::mem::take(&mut self.int32),
            data_int64: std::mem::take(&mut self.int64),
            data_double: std::mem::take(&mut self.doubles),
            row_number: std::mem::take(&mut self.row_numbers),
        };
        self.clear();
        table
    }

    pub fn clear(&mut self) {
        self.flush_count += 1;
        self.strings.clear();
        self.nulls.clear();
        self.bools.clear();
        self.int32.clear();
        self.int64.clear();
        self.doubles.clear();
        self.row_numbers.clear();
        self.rows = 0;
        self.byte_estimate = 0;
    }

    pub fn len(&self) -> usize {
        self.rows
    }

    pub fn is_empty(&self) -> bool {
        self.rows == 0
    }

    /// Emit a `debug!` event with RSS, buffer state, and cumulative counters.
    ///
    /// Called at `MT_EMIT` flush points (threshold flush and end-of-run flush)
    /// and at row-count checkpoints from `push`. At flush points this is called
    /// before `clear()`, so `flush_count` reflects completed flushes; the event
    /// reports `flush_count + 1` — the 1-indexed number of the flush about to
    /// happen. At checkpoint calls the `+ 1` anticipates the next flush, which is
    /// the same convention (the checkpoint fires mid-accumulation, not on a flush).
    /// Suppressed automatically when the resolved tracing level is above `debug`.
    pub fn record_flush_telemetry(&self) {
        tracing::debug!(
            target: "emit_flush",
            rss_kb = read_rss_kb(),
            byte_estimate = self.byte_estimate,
            cumulative_bytes = self.cumulative_bytes,
            cumulative_rows = self.cumulative_rows,
            flush_count = self.flush_count + 1,
            buffered_rows = self.rows,
            "MT_EMIT flush"
        );
    }

    /// Append an Arrow `RecordBatch` to the emit stream, flushing whenever the
    /// buffer reaches `EMIT_BUFFER_LIMIT_BYTES` at a row boundary.
    ///
    /// Each column array is downcast and its null buffer read once, then the
    /// rows are packed into the same blocks the row path fills — no `Value` row
    /// is materialised, and rows a preceding `emit` left buffered are not
    /// displaced. A batch that cannot reach the threshold skips the per-row cost
    /// vector for the O(columns) estimate.
    ///
    /// Every row of the batch belongs to the input row being processed, so all
    /// of them carry `row_number`.
    #[cfg(feature = "emit-arrow")]
    pub fn push_batch(
        &mut self,
        batch: &arrow::record_batch::RecordBatch,
        meta: &[ColumnInfo],
        row_number: u64,
        flush: &mut dyn FnMut(exa_proto::ExascriptTableData) -> Result<(), UdfError>,
    ) -> Result<(), UdfError> {
        let n_rows = batch.num_rows();
        if n_rows == 0 {
            return Ok(());
        }

        // Fail before any row of this batch lands in the blocks.
        let accessors = build_accessors(batch, meta)?;
        let nulls: Vec<_> = (0..meta.len())
            .map(|c| batch.column(c).nulls().cloned())
            .collect();

        let batch_cost = batch_byte_cost(batch, meta) + n_rows * BYTES_ROW_NUMBER;
        if self.byte_estimate + batch_cost < EMIT_BUFFER_LIMIT_BYTES {
            self.account(n_rows as u64, batch_cost);
            for r in 0..n_rows {
                self.push_arrow_row(&accessors, &nulls, r, row_number);
            }
            return Ok(());
        }

        let row_costs = compute_row_costs(batch, meta);
        for (r, &row_cost) in row_costs.iter().enumerate() {
            self.account(1, row_cost + BYTES_ROW_NUMBER);
            self.push_arrow_row(&accessors, &nulls, r, row_number);
            if self.should_flush() {
                self.record_flush_telemetry();
                flush(self.take_proto())?;
            }
        }
        Ok(())
    }

    /// Pack row `r` of a downcast batch into the blocks, mirroring
    /// `push_costed`'s per-cell layout. The accessor variants are listed
    /// exhaustively so a new `ColAccessor` must choose its block here instead of
    /// silently landing in the string block.
    #[cfg(feature = "emit-arrow")]
    fn push_arrow_row(
        &mut self,
        accessors: &[ColAccessor<'_>],
        nulls: &[Option<arrow::buffer::NullBuffer>],
        r: usize,
        row_number: u64,
    ) {
        for (c, acc) in accessors.iter().enumerate() {
            if nulls[c].as_ref().is_some_and(|nb| nb.is_null(r)) {
                self.nulls.push(true);
                continue;
            }
            self.nulls.push(false);
            match acc {
                ColAccessor::Int32(arr) => self.int32.push(arr.value(r)),
                ColAccessor::Int64(arr) => self.int64.push(arr.value(r)),
                ColAccessor::Float64(arr) => self.doubles.push(arr.value(r)),
                ColAccessor::Boolean(arr) => self.bools.push(arr.value(r)),
                ColAccessor::Utf8(arr) => self.strings.push(arr.value(r).to_string()),
                ColAccessor::LargeUtf8(arr) => self.strings.push(arr.value(r).to_string()),
                ColAccessor::Date32(_)
                | ColAccessor::TsSecond(_)
                | ColAccessor::TsMillisecond(_)
                | ColAccessor::TsMicrosecond(_)
                | ColAccessor::TsNanosecond(_)
                | ColAccessor::Decimal128(_, _)
                | ColAccessor::NumericFromInt32(_)
                | ColAccessor::NumericFromInt64(_)
                | ColAccessor::NumericFromFloat64(_) => self
                    .strings
                    .push(value_into_block_string(accessor_value(acc, r))),
                ColAccessor::Unsupported => {}
            }
        }
        self.row_numbers.push(row_number);
        self.rows += 1;
    }
}

/// A typed accessor for one Arrow column that has been downcast exactly once.
///
/// Built by `build_accessors` before any row-level encoding begins. The inner
/// reference borrows from the `RecordBatch` that owns the column buffers, so
/// all accessor lifetimes are tied to the batch's lifetime.
///
/// The variant chosen records both the Arrow type (which determines how to
/// extract a cell value) and — for the widening cases — the declared
/// `ExaType` target (which determines which proto block the value lands in).
/// The `ExaType` authority is the declared `ColumnInfo`; the Arrow type is
/// used only for extraction.
#[cfg(feature = "emit-arrow")]
enum ColAccessor<'a> {
    Int32(&'a arrow::array::Int32Array),
    Int64(&'a arrow::array::Int64Array),
    Float64(&'a arrow::array::Float64Array),
    Boolean(&'a arrow::array::BooleanArray),
    Utf8(&'a arrow::array::StringArray),
    LargeUtf8(&'a arrow::array::LargeStringArray),
    Date32(&'a arrow::array::Date32Array),
    TsSecond(&'a arrow::array::TimestampSecondArray),
    TsMillisecond(&'a arrow::array::TimestampMillisecondArray),
    TsMicrosecond(&'a arrow::array::TimestampMicrosecondArray),
    TsNanosecond(&'a arrow::array::TimestampNanosecondArray),
    Decimal128(&'a arrow::array::Decimal128Array, i8),
    /// Int32/Int64/Float64 Arrow column declared as `ExaType::Numeric` (BIGINT
    /// widening): extract value as the natural type; `push_arrow_row`
    /// stringifies it into the string block via `value_into_block_string`.
    NumericFromInt32(&'a arrow::array::Int32Array),
    NumericFromInt64(&'a arrow::array::Int64Array),
    NumericFromFloat64(&'a arrow::array::Float64Array),
    Unsupported,
}

/// Downcast each column of `batch` to its concrete Arrow array type exactly
/// once, validate the (Arrow type, declared ExaType) combination, and return
/// a per-column `ColAccessor` vec.
///
/// Validation and downcast are merged into one pass so the row-level encoding
/// loop has no validation branches and no `unreachable!` arms.
#[cfg(feature = "emit-arrow")]
fn build_accessors<'a>(
    batch: &'a arrow::record_batch::RecordBatch,
    meta: &[ColumnInfo],
) -> Result<Vec<ColAccessor<'a>>, UdfError> {
    use arrow::array::{
        Array, BooleanArray, Date32Array, Decimal128Array, Float64Array, Int32Array, Int64Array,
        LargeStringArray, StringArray, TimestampMicrosecondArray, TimestampMillisecondArray,
        TimestampNanosecondArray, TimestampSecondArray,
    };
    use arrow::datatypes::{DataType, TimeUnit};

    if batch.num_columns() != meta.len() {
        return Err(UdfError::Type(format!(
            "emit_batch: batch has {} columns but EMITS declared {} columns",
            batch.num_columns(),
            meta.len()
        )));
    }

    let mut accessors = Vec::with_capacity(meta.len());

    for (c, col_meta) in meta.iter().enumerate() {
        let col = batch.column(c);
        let dt = col.data_type();
        let typ = &col_meta.typ;

        let acc = match (dt, typ) {
            (DataType::Int32, ExaType::Int32) => {
                ColAccessor::Int32(col.as_any().downcast_ref::<Int32Array>().unwrap())
            }
            (DataType::Int64, ExaType::Int64) => {
                ColAccessor::Int64(col.as_any().downcast_ref::<Int64Array>().unwrap())
            }
            (DataType::Float64, ExaType::Double) => {
                ColAccessor::Float64(col.as_any().downcast_ref::<Float64Array>().unwrap())
            }
            (DataType::Boolean, ExaType::Boolean) => {
                ColAccessor::Boolean(col.as_any().downcast_ref::<BooleanArray>().unwrap())
            }
            (DataType::Utf8, typ) if is_string_family_exatype(typ) => {
                ColAccessor::Utf8(col.as_any().downcast_ref::<StringArray>().unwrap())
            }
            (DataType::LargeUtf8, typ) if is_string_family_exatype(typ) => {
                ColAccessor::LargeUtf8(col.as_any().downcast_ref::<LargeStringArray>().unwrap())
            }
            (DataType::Date32, ExaType::Date) => {
                ColAccessor::Date32(col.as_any().downcast_ref::<Date32Array>().unwrap())
            }
            (DataType::Timestamp(unit, _), ExaType::Timestamp { .. }) => match unit {
                TimeUnit::Second => ColAccessor::TsSecond(
                    col.as_any().downcast_ref::<TimestampSecondArray>().unwrap(),
                ),
                TimeUnit::Millisecond => ColAccessor::TsMillisecond(
                    col.as_any()
                        .downcast_ref::<TimestampMillisecondArray>()
                        .unwrap(),
                ),
                TimeUnit::Microsecond => ColAccessor::TsMicrosecond(
                    col.as_any()
                        .downcast_ref::<TimestampMicrosecondArray>()
                        .unwrap(),
                ),
                TimeUnit::Nanosecond => ColAccessor::TsNanosecond(
                    col.as_any()
                        .downcast_ref::<TimestampNanosecondArray>()
                        .unwrap(),
                ),
            },
            (DataType::Decimal128(_, scale), ExaType::Numeric { .. }) => ColAccessor::Decimal128(
                col.as_any().downcast_ref::<Decimal128Array>().unwrap(),
                *scale,
            ),
            (DataType::Int32, ExaType::Numeric { .. }) => {
                ColAccessor::NumericFromInt32(col.as_any().downcast_ref::<Int32Array>().unwrap())
            }
            (DataType::Int64, ExaType::Numeric { .. }) => {
                ColAccessor::NumericFromInt64(col.as_any().downcast_ref::<Int64Array>().unwrap())
            }
            (DataType::Float64, ExaType::Numeric { .. }) => ColAccessor::NumericFromFloat64(
                col.as_any().downcast_ref::<Float64Array>().unwrap(),
            ),
            (_, ExaType::Unsupported) => ColAccessor::Unsupported,
            _ => {
                return Err(UdfError::Type(format!(
                    "emit_batch: Arrow column {c} of type {dt:?} cannot feed declared ExaType {typ:?}"
                )));
            }
        };
        accessors.push(acc);
    }

    Ok(accessors)
}

/// Returns true for any `ExaType` that maps to the string proto block.
#[cfg(feature = "emit-arrow")]
fn is_string_family_exatype(typ: &ExaType) -> bool {
    matches!(
        typ,
        ExaType::Numeric { .. }
            | ExaType::Date
            | ExaType::Timestamp { .. }
            | ExaType::String { .. }
            | ExaType::Char { .. }
    )
}

/// 1970-01-01 as a CE day number: an Arrow `Date32` becomes a `NaiveDate` by
/// adding it.
#[cfg(feature = "emit-arrow")]
const ARROW_EPOCH_CE_DAY: i32 = 719163;

/// Convert one non-NULL Arrow cell to the SDK `Value` the row path would have
/// carried for it.
///
/// The single owner of the Arrow decoding decisions — the `Date32` CE-day
/// offset, each `Timestamp` unit's divisor (euclidean, so a pre-epoch negative
/// count still yields the non-negative sub-second remainder `chrono` requires),
/// and the `Decimal128` unscaled/scale pair. `push_arrow_row` is its only
/// consumer and may not re-derive any of it, or the batch and row paths can
/// disagree and break byte-identity.
///
/// `row` must index a non-NULL cell — callers read nullness in bulk per column.
/// `ColAccessor::Unsupported` yields `Value::Null`.
#[cfg(feature = "emit-arrow")]
fn accessor_value(acc: &ColAccessor<'_>, row: usize) -> Value {
    match acc {
        ColAccessor::Int32(arr) => Value::Int32(arr.value(row)),
        ColAccessor::Int64(arr) => Value::Int64(arr.value(row)),
        ColAccessor::Float64(arr) => Value::Double(arr.value(row)),
        ColAccessor::Boolean(arr) => Value::Bool(arr.value(row)),
        ColAccessor::Utf8(arr) => Value::String(arr.value(row).to_string()),
        ColAccessor::LargeUtf8(arr) => Value::String(arr.value(row).to_string()),
        ColAccessor::Date32(arr) => Value::Date(
            NaiveDate::from_num_days_from_ce_opt(arr.value(row) + ARROW_EPOCH_CE_DAY)
                .unwrap_or_default(),
        ),
        ColAccessor::TsSecond(arr) => Value::Timestamp(
            chrono::DateTime::from_timestamp(arr.value(row), 0)
                .map(|dt| dt.naive_utc())
                .unwrap_or_default(),
        ),
        ColAccessor::TsMillisecond(arr) => Value::Timestamp(
            chrono::DateTime::from_timestamp_millis(arr.value(row))
                .map(|dt| dt.naive_utc())
                .unwrap_or_default(),
        ),
        ColAccessor::TsMicrosecond(arr) => Value::Timestamp(
            chrono::DateTime::from_timestamp_micros(arr.value(row))
                .map(|dt| dt.naive_utc())
                .unwrap_or_default(),
        ),
        ColAccessor::TsNanosecond(arr) => {
            let ns = arr.value(row);
            Value::Timestamp(
                chrono::DateTime::from_timestamp(
                    ns.div_euclid(1_000_000_000),
                    ns.rem_euclid(1_000_000_000) as u32,
                )
                .map(|dt| dt.naive_utc())
                .unwrap_or_default(),
            )
        }
        ColAccessor::Decimal128(arr, scale) => Value::Numeric(Decimal {
            unscaled: arr.value(row),
            scale: *scale as u8,
        }),
        ColAccessor::NumericFromInt32(arr) => Value::Int32(arr.value(row)),
        ColAccessor::NumericFromInt64(arr) => Value::Int64(arr.value(row)),
        ColAccessor::NumericFromFloat64(arr) => Value::Double(arr.value(row)),
        ColAccessor::Unsupported => Value::Null,
    }
}

/// Fixed per-cell byte cost for an Arrow `DataType` of constant width — the
/// batch-path counterpart of `value_byte_cost`. `None` for variable-width types
/// (`Utf8`/`LargeUtf8`) and for anything this path does not cost.
#[cfg(feature = "emit-arrow")]
fn fixed_cell_cost(dt: &arrow::datatypes::DataType) -> Option<usize> {
    use arrow::datatypes::DataType;
    match dt {
        DataType::Boolean => Some(BYTES_BOOL),
        DataType::Int32 => Some(BYTES_INT32),
        DataType::Int64 => Some(BYTES_INT64),
        DataType::Float64 => Some(BYTES_DOUBLE),
        DataType::Date32 => Some(BYTES_DATE),
        DataType::Timestamp(_, _) => Some(BYTES_TIMESTAMP),
        DataType::Decimal128(_, scale) => Some(NUMERIC_COST_BASE + *scale as usize),
        _ => None,
    }
}

/// Add `cell_cost(row)` to `costs[row]` for every non-NULL row — a NULL cell
/// occupies no type-block slot, matching `value_byte_cost`.
#[cfg(feature = "emit-arrow")]
fn accumulate_costs(
    costs: &mut [usize],
    nulls: Option<&arrow::buffer::NullBuffer>,
    cell_cost: impl Fn(usize) -> usize,
) {
    for (r, cost) in costs.iter_mut().enumerate() {
        if !nulls.is_some_and(|nb| nb.is_null(r)) {
            *cost += cell_cost(r);
        }
    }
}

/// Total byte cost of a whole batch in O(columns): fixed-width columns from
/// their null count, variable-width columns from the offset buffer's span.
/// An upper bound on the per-row sum, so it can only flush early.
#[cfg(feature = "emit-arrow")]
fn batch_byte_cost(batch: &arrow::record_batch::RecordBatch, meta: &[ColumnInfo]) -> usize {
    use arrow::array::{Array, LargeStringArray, StringArray};
    use arrow::datatypes::DataType;

    let n_rows = batch.num_rows();
    let mut total = 0usize;

    for c in 0..meta.len() {
        let col = batch.column(c);
        if let Some(fixed) = fixed_cell_cost(col.data_type()) {
            total += fixed * (n_rows - col.null_count());
            continue;
        }
        total += match col.data_type() {
            DataType::Utf8 => col
                .as_any()
                .downcast_ref::<StringArray>()
                .map_or(0, |arr| offsets_span(arr.offsets())),
            DataType::LargeUtf8 => col
                .as_any()
                .downcast_ref::<LargeStringArray>()
                .map_or(0, |arr| offsets_span(arr.offsets())),
            // Unsupported: cost 0 (type validation ran before this call).
            _ => 0,
        };
    }

    total
}

#[cfg(feature = "emit-arrow")]
fn offsets_span<O: arrow::array::OffsetSizeTrait>(
    offsets: &arrow::buffer::OffsetBuffer<O>,
) -> usize {
    let o = offsets.inner();
    (o[o.len() - 1] - o[0]).as_usize()
}

/// Compute a per-row byte cost vector for the batch using Arrow's columnar
/// layout for efficiency (no per-cell work for fixed-width types; offset-buffer
/// deltas for variable-width; same fixed estimates as `value_byte_cost`).
#[cfg(feature = "emit-arrow")]
fn compute_row_costs(batch: &arrow::record_batch::RecordBatch, meta: &[ColumnInfo]) -> Vec<usize> {
    use arrow::array::{Array, LargeStringArray, StringArray};
    use arrow::datatypes::DataType;

    let n_rows = batch.num_rows();
    let mut costs = vec![0usize; n_rows];

    for c in 0..meta.len() {
        let col = batch.column(c);
        let null_buf = col.nulls();

        if let Some(fixed) = fixed_cell_cost(col.data_type()) {
            accumulate_costs(&mut costs, null_buf, |_r| fixed);
            continue;
        }

        match col.data_type() {
            DataType::Utf8 => {
                if let Some(arr) = col.as_any().downcast_ref::<StringArray>() {
                    let o = arr.offsets();
                    accumulate_costs(&mut costs, null_buf, |r| (o[r + 1] - o[r]) as usize);
                }
            }
            DataType::LargeUtf8 => {
                if let Some(arr) = col.as_any().downcast_ref::<LargeStringArray>() {
                    let o = arr.offsets();
                    accumulate_costs(&mut costs, null_buf, |r| (o[r + 1] - o[r]) as usize);
                }
            }
            _ => {
                // Unsupported: cost 0 (type validation ran before this call).
            }
        }
    }

    costs
}

/// Wire formats for the string-block temporal types. The DB serialises DATE as
/// `YYYY-MM-DD` and TIMESTAMP as `YYYY-MM-DD HH:MM:SS.ffffff` (space separator).
const DATE_FORMAT: &str = "%Y-%m-%d";
/// Parse format: `%.f` is optional fractional digits, tolerates both `HH:MM:SS` and `HH:MM:SS.ffffff`.
const TIMESTAMP_PARSE: &str = "%Y-%m-%d %H:%M:%S%.f";
/// Emit format: `%.9f` always emits exactly 9 fractional (nanosecond) digits.
/// The Exasol engine truncates the emitted value to the output column's declared
/// precision on receipt (`SWIGResultHandler::setTimestamp` parses `YYYY-MM-DD
/// HH24:MI:SS.FF9` then applies `trunc_to_fractional_seconds_precision(value,
/// m_types[col].prec)`), so emitting all 9 digits is lossless for every declared
/// precision; the old `%.6f` capped output at microseconds and lost precision for
/// `TIMESTAMP(7/8/9)`. This benefits only UDF-*generated* sub-microsecond values
/// (e.g. a wall-clock or connect-back source): the DB delivers input columns to
/// every UDF at microsecond precision (`SWIGTableData::getTimestamp` formats
/// `...FF6`), so an input→output round-trip is capped at microseconds regardless
/// of this emit format.
const TIMESTAMP_EMIT: &str = "%Y-%m-%d %H:%M:%S%.9f";
/// ISO-8601 `T`-separated fallback some sources emit for timestamps.
const TIMESTAMP_FORMAT_ISO: &str = "%Y-%m-%dT%H:%M:%S%.f";

/// Parse an exactly-2-digit ASCII decimal field (e.g. `MM`, `DD`, `HH`, `MI`,
/// `SS`), returning `None` for anything that is not two ASCII digits.
fn parse_2digit(b: &[u8]) -> Option<u32> {
    if b.len() != 2 || !b[0].is_ascii_digit() || !b[1].is_ascii_digit() {
        return None;
    }
    Some((b[0] - b'0') as u32 * 10 + (b[1] - b'0') as u32)
}

/// Parse an exactly-4-digit ASCII decimal field (`YYYY`), returning `None` for
/// anything that is not four ASCII digits.
fn parse_4digit(b: &[u8]) -> Option<u32> {
    if b.len() != 4 {
        return None;
    }
    let mut v = 0u32;
    for &c in b {
        if !c.is_ascii_digit() {
            return None;
        }
        v = v * 10 + (c - b'0') as u32;
    }
    Some(v)
}

/// Hand-rolled fixed-format parser for the DATE wire form `YYYY-MM-DD`,
/// replacing `NaiveDate::parse_from_str`'s generic strptime-style interpreter
/// with direct byte-position digit reads (the mirror image of
/// `fast_date_to_string`). Scoped to the exact 10-byte fixed-width layout the
/// DB always sends (see the module doc comment above `DATE_FORMAT`); anything
/// that doesn't match this exact shape (non-standard digit widths, wrong
/// separators, garbage) returns `None` so the caller falls back to
/// `NaiveDate::parse_from_str`, preserving that path's leniency exactly (see
/// `decode_string_block_preserves_leniency_when_fast_path_defers`). Verified
/// byte-identical to the chrono path by
/// `fast_string_block_ingest_tests::fast_parse_date_matches_chrono_parse_for_valid_dates`.
fn fast_parse_date(s: &str) -> Option<NaiveDate> {
    let b = s.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    let year = parse_4digit(&b[0..4])?;
    let month = parse_2digit(&b[5..7])?;
    let day = parse_2digit(&b[8..10])?;
    NaiveDate::from_ymd_opt(year as i32, month, day)
}

/// Hand-rolled fixed-format parser for the TIMESTAMP wire form
/// `YYYY-MM-DD HH:MM:SS[.f]` (0 to 9 fractional digits; also accepts the
/// `T`-separated ISO variant), replacing the two generic
/// `NaiveDateTime::parse_from_str` attempts with direct byte-position digit
/// reads — the mirror image of `fast_timestamp_to_string`. Anything that
/// doesn't match this exact fixed shape (non-standard digit widths, a leap
/// second, more than 9 fractional digits, an unrecognised separator, garbage)
/// returns `None` so the caller falls back to the existing two-attempt chrono
/// chain, preserving that path's leniency exactly (see
/// `decode_string_block_preserves_leniency_when_fast_path_defers`). Verified
/// byte-identical to the chrono path by
/// `fast_string_block_ingest_tests::fast_parse_timestamp_matches_chrono_parse_for_valid_timestamps`.
fn fast_parse_timestamp(s: &str) -> Option<NaiveDateTime> {
    let b = s.as_bytes();
    if b.len() < 19 || b[4] != b'-' || b[7] != b'-' || b[13] != b':' || b[16] != b':' {
        return None;
    }
    let sep = b[10];
    if sep != b' ' && sep != b'T' {
        return None;
    }

    let year = parse_4digit(&b[0..4])?;
    let month = parse_2digit(&b[5..7])?;
    let day = parse_2digit(&b[8..10])?;
    let hour = parse_2digit(&b[11..13])?;
    let minute = parse_2digit(&b[14..16])?;
    let second = parse_2digit(&b[17..19])?;

    let nanos = match b.len() {
        19 => 0u32,
        len if len > 20 && b[19] == b'.' => {
            let frac = &b[20..];
            if frac.is_empty() || frac.len() > 9 || !frac.iter().all(u8::is_ascii_digit) {
                return None;
            }
            let mut value = 0u32;
            for &c in frac {
                value = value * 10 + (c - b'0') as u32;
            }
            value * 10u32.pow(9 - frac.len() as u32)
        }
        _ => return None,
    };

    let date = NaiveDate::from_ymd_opt(year as i32, month, day)?;
    date.and_hms_nano_opt(hour, minute, second, nanos)
}

/// Decode one non-null `data_string` cell into its typed `Value` per the column
/// type. NUMERIC/DATE/TIMESTAMP parse into their typed payloads; a parse failure
/// yields `Value::Null` so corrupt wire data stays decodable rather than
/// aborting the whole batch. Extended string-backed types pass through verbatim.
///
/// DATE/TIMESTAMP first try the hand-rolled fixed-format parsers above
/// (`fast_parse_date`/`fast_parse_timestamp`), falling back to the original
/// `chrono::parse_from_str` chain for anything outside their fixed-width
/// scope — the same byte-identical-with-fallback shape as the emit-side
/// `value_to_block_string` fast formatters.
fn decode_string_block(typ: &ExaType, s: &str) -> Value {
    match typ {
        ExaType::Numeric { .. } => match Decimal::try_from(s) {
            Ok(d) => Value::Numeric(d),
            Err(_) => Value::Null,
        },
        ExaType::Date => {
            match fast_parse_date(s).or_else(|| NaiveDate::parse_from_str(s, DATE_FORMAT).ok()) {
                Some(d) => Value::Date(d),
                None => Value::Null,
            }
        }
        ExaType::Timestamp { .. } => {
            match fast_parse_timestamp(s)
                .or_else(|| NaiveDateTime::parse_from_str(s, TIMESTAMP_PARSE).ok())
                .or_else(|| NaiveDateTime::parse_from_str(s, TIMESTAMP_FORMAT_ISO).ok())
            {
                Some(ts) => Value::Timestamp(ts),
                None => Value::Null,
            }
        }
        _ => Value::String(s.to_string()),
    }
}

/// Hand-rolled digit-writer replacing `Decimal`'s `Display` impl for the
/// emit-side string block. `itoa::Buffer::format` writes the `i128`/`u128`
/// digit run into a stack buffer with no intermediate `String`
/// allocation-then-reparse; the decimal point is then spliced in directly.
/// Mirrors `Decimal::fmt` exactly (see `value.rs`), verified byte-identical by
/// `fast_string_block_tests::fast_decimal_matches_display_for_all_cases`.
fn fast_decimal_to_string(d: &Decimal) -> String {
    let mut buf = itoa::Buffer::new();
    if d.scale == 0 {
        return buf.format(d.unscaled).to_string();
    }

    let negative = d.unscaled < 0;
    let digits = buf.format(d.unscaled.unsigned_abs());
    let scale = d.scale as usize;

    let mut out = String::with_capacity(digits.len() + scale + 2);
    if negative {
        out.push('-');
    }
    if digits.len() <= scale {
        out.push_str("0.");
        for _ in 0..(scale - digits.len()) {
            out.push('0');
        }
        out.push_str(digits);
    } else {
        let point = digits.len() - scale;
        out.push_str(&digits[..point]);
        out.push('.');
        out.push_str(&digits[point..]);
    }
    out
}

/// Write a zero-padded 2-digit decimal number (0..=99) directly as ASCII
/// bytes, avoiding `core::fmt`'s width/padding machinery.
fn push_2digit(out: &mut String, v: u32) {
    out.push((b'0' + (v / 10) as u8) as char);
    out.push((b'0' + (v % 10) as u8) as char);
}

/// Write a zero-padded `width`-digit decimal number as ASCII bytes via plain
/// division/modulo — a fixed-width zero-padded digit writer with no runtime
/// format-string interpretation.
fn push_ndigit(out: &mut String, v: u32, width: u32) {
    let mut divisor = 10u32.pow(width - 1);
    let mut remaining = v;
    for _ in 0..width {
        out.push((b'0' + (remaining / divisor) as u8) as char);
        remaining %= divisor;
        divisor /= 10;
    }
}

/// Fast `YYYY-MM-DD` formatter for `NaiveDate`, replacing chrono's generic
/// `.format()` (which re-parses the `"%Y-%m-%d"` pattern on every call) with
/// direct accessor reads (`year()`/`month()`/`day()` are O(1)) and hand-rolled
/// zero-padded digit writes.
///
/// Scoped to the common case: years in `0..=9999` render as a zero-padded
/// 4-digit field, matching chrono's `%Y` for that range exactly (verified in
/// `fast_date_matches_chrono_format`). Outside that range chrono renders a
/// variable-width `+`/`-`-prefixed field instead (see
/// `fast_date_defers_for_out_of_common_range_years`); Exasol's DATE type only
/// ever carries `0001-01-01..=9999-12-31`, so this covers every value that can
/// actually reach the wire. Returns `None` for out-of-range years so the
/// caller falls back to `NaiveDate::format`, preserving byte-identical output
/// for every representable date.
fn fast_date_to_string(d: &NaiveDate) -> Option<String> {
    use chrono::Datelike;

    let year = d.year();
    if !(0..=9999).contains(&year) {
        return None;
    }

    let mut out = String::with_capacity(10);
    push_ndigit(&mut out, year as u32, 4);
    out.push('-');
    push_2digit(&mut out, d.month());
    out.push('-');
    push_2digit(&mut out, d.day());
    Some(out)
}

/// Fast `YYYY-MM-DD HH:MM:SS.fffffffff` formatter for `NaiveDateTime`,
/// replacing chrono's generic `.format()` the same way `fast_date_to_string`
/// does for the date part, plus hand-rolled zero-padded time and always-9-digit
/// nanosecond fields.
///
/// Defers to `None` (letting the caller fall back to `NaiveDateTime::format`)
/// when the date part is out of the common year range (see
/// `fast_date_to_string`) or when `nanosecond()` reports a leap-second value
/// (`>= 1_000_000_000`, per chrono's `Timelike::nanosecond` docs) — an edge
/// case Exasol TIMESTAMP values never produce, kept out of the fast path
/// rather than reverse-engineering chrono's undocumented leap-second
/// rendering.
fn fast_timestamp_to_string(ts: &NaiveDateTime) -> Option<String> {
    use chrono::Timelike;

    let date_part = fast_date_to_string(&ts.date())?;
    let nanos = ts.nanosecond();
    if nanos >= 1_000_000_000 {
        return None;
    }

    let mut out = String::with_capacity(29);
    out.push_str(&date_part);
    out.push(' ');
    push_2digit(&mut out, ts.hour());
    out.push(':');
    push_2digit(&mut out, ts.minute());
    out.push(':');
    push_2digit(&mut out, ts.second());
    out.push('.');
    push_ndigit(&mut out, nanos, 9);
    Some(out)
}

/// Render a non-null `Value` as the text form for a string/numeric/temporal
/// block. Typed variants are serialised back to their wire form; numeric integer
/// and double variants are stringified so a DECIMAL EMITS column receiving a
/// `Value::Int64`/`Value::Double` from a connect-back SELECT still serialises.
///
/// NUMERIC/DATE/TIMESTAMP use the hand-rolled fast formatters above, falling
/// back to the `chrono`/`Display` path for the (rare, out-of-Exasol-range)
/// cases they defer on — see `fast_date_to_string`/`fast_timestamp_to_string`.
/// The `fast_string_block_tests` regression suite proves this is byte-identical
/// to the `chrono`/`Display` path for every representable value.
fn value_to_block_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Numeric(d) => fast_decimal_to_string(d),
        Value::Date(d) => {
            fast_date_to_string(d).unwrap_or_else(|| d.format(DATE_FORMAT).to_string())
        }
        Value::Timestamp(ts) => {
            fast_timestamp_to_string(ts).unwrap_or_else(|| ts.format(TIMESTAMP_EMIT).to_string())
        }
        Value::Int32(i) => i.to_string(),
        Value::Int64(i) => i.to_string(),
        Value::Double(f) => f.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => String::new(),
    }
}

/// `value_to_block_string` for a `Value` the caller owns, so a `Value::String`
/// moves into the block instead of being cloned. Same bytes for every variant.
#[cfg(feature = "emit-arrow")]
fn value_into_block_string(v: Value) -> String {
    match v {
        Value::String(s) => s,
        other => value_to_block_string(&other),
    }
}

/// `value_to_block_string` for a cell the caller is about to discard: a
/// `Value::String`'s buffer moves into the block, leaving an empty string
/// behind. Every other variant formats exactly as the borrowing form, with no
/// move — the cell is left as it was.
fn value_take_block_string(v: &mut Value) -> String {
    match v {
        Value::String(s) => std::mem::take(s),
        other => value_to_block_string(other),
    }
}

/// Coerce a non-null `Value` to `i64` for an INT32/INT64 EMITS column.
/// Reject an output row that the declared columns cannot carry losslessly, and
/// return its buffered byte cost from the same pass.
///
/// `take_proto` packs by declared column type, so an arity or variant mismatch
/// would otherwise land as a NULL, a truncated number or a stringified variant
/// with no error anywhere: the DB acknowledges `MT_EMIT` before it reads the
/// row and never reports a per-row problem. NULL is valid in every column.
fn check_output_row(row: &[Value], meta: &[ColumnInfo]) -> Result<usize, UdfError> {
    if row.len() != meta.len() {
        return Err(UdfError::Type(format!(
            "output row has {} value(s) but the output has {} column(s)",
            row.len(),
            meta.len()
        )));
    }
    let mut cost = BYTES_ROW_NUMBER;
    for (idx, (v, col)) in row.iter().zip(meta).enumerate() {
        if !column_accepts(&col.typ, v) {
            return Err(UdfError::Type(format!(
                "output column {idx} `{}` is {} but the value is {v:?}",
                col.name, col.type_name
            )));
        }
        cost += value_byte_cost(v);
    }
    Ok(cost)
}

/// Whether a value can feed a column of this declared type. `Int64` into an
/// INT32 column is range-checked; every other pairing is decided by variant.
fn column_accepts(typ: &ExaType, v: &Value) -> bool {
    match (typ, v) {
        (_, Value::Null) | (ExaType::Unsupported, _) => true,
        (ExaType::Int32, Value::Int32(_)) => true,
        (ExaType::Int32, Value::Int64(i)) => i32::try_from(*i).is_ok(),
        (ExaType::Int64 | ExaType::Numeric { .. }, Value::Int32(_) | Value::Int64(_)) => true,
        (ExaType::Numeric { .. }, Value::Numeric(_)) => true,
        (ExaType::Double, Value::Double(_)) => true,
        (ExaType::Boolean, Value::Bool(_)) => true,
        (ExaType::Date, Value::Date(_)) => true,

        (ExaType::Timestamp { .. }, Value::Timestamp(_)) => true,
        (ExaType::String { .. } | ExaType::Char { .. }, Value::String(_)) => true,
        _ => false,
    }
}

fn value_to_i64(v: &Value) -> i64 {
    match v {
        Value::Int32(i) => *i as i64,
        Value::Int64(i) => *i,
        Value::Double(f) => *f as i64,
        Value::Numeric(d) => {
            let scaled = d.unscaled / 10i128.pow(d.scale as u32);
            i64::try_from(scaled).unwrap_or(0)
        }
        Value::String(s) => s.parse().unwrap_or(0),
        Value::Bool(b) => *b as i64,
        _ => 0,
    }
}

/// Coerce a non-null `Value` to `f64` for a DOUBLE EMITS column.
fn value_to_f64(v: &Value) -> f64 {
    match v {
        Value::Double(f) => *f,
        Value::Int32(i) => *i as f64,
        Value::Int64(i) => *i as f64,
        Value::Numeric(d) => d.unscaled as f64 / 10f64.powi(d.scale as i32),
        Value::String(s) => s.parse().unwrap_or(0.0),
        _ => 0.0,
    }
}

/// Coerce a non-null `Value` to `bool` for a BOOLEAN EMITS column.
fn value_to_bool(v: &Value) -> bool {
    match v {
        Value::Bool(b) => *b,
        Value::Int32(i) => *i != 0,
        Value::Int64(i) => *i != 0,
        Value::Numeric(d) => d.unscaled != 0,
        Value::String(s) => s == "true" || s == "TRUE" || s == "1",
        _ => false,
    }
}

/// Return the first non-loopback IPv4 address found on the local network
/// interfaces by walking the `getifaddrs` linked list.
///
/// Uses `libc::getifaddrs` because the UDF process is a normal Linux process
/// inside the Exasol container with full access to interface-enumeration
/// syscalls, and this approach works on single-node Docker (where the ZMQ
/// endpoint is `ipc://`) and multi-node TCP clusters alike.
#[cfg(feature = "connect-back")]
fn first_nonloopback_ipv4() -> Result<String, exasol_udf_sdk::error::UdfError> {
    use exasol_udf_sdk::error::UdfError;

    // Safety: `getifaddrs` is a POSIX syscall. `ifap` is only accessed inside
    // this function and freed before return. No Rust references alias the raw
    // pointer during traversal.
    let mut ifap: *mut libc::ifaddrs = std::ptr::null_mut();
    let rc = unsafe { libc::getifaddrs(&mut ifap) };
    if rc != 0 {
        return Err(UdfError::ConnectBack(format!(
            "getifaddrs failed with return code {rc}"
        )));
    }

    let mut result: Option<String> = None;

    // Walk the singly-linked list of interfaces.
    let mut ifa = ifap;
    while !ifa.is_null() {
        // Safety: `ifa` is a valid pointer produced by `getifaddrs`.
        let flags = unsafe { (*ifa).ifa_flags };
        let addr_ptr = unsafe { (*ifa).ifa_addr };

        // Skip interfaces that are not up or have no address.
        if flags & libc::IFF_UP as u32 != 0 && !addr_ptr.is_null() {
            // Safety: `addr_ptr` is non-null and valid per `getifaddrs` contract.
            let family = unsafe { (*addr_ptr).sa_family };
            if family == libc::AF_INET as libc::sa_family_t {
                // Safety: family is AF_INET so the pointer refers to a sockaddr_in.
                let sin = addr_ptr as *const libc::sockaddr_in;
                let s_addr = unsafe { (*sin).sin_addr.s_addr };

                // `s_addr` is in network byte order (big-endian). Converting to
                // host order and extracting the high byte gives the first IP
                // octet; 127 means loopback (127.0.0.0/8).
                let octets = u32::from_be(s_addr).to_be_bytes();
                if octets[0] != 127 {
                    result = Some(format!(
                        "{}.{}.{}.{}",
                        octets[0], octets[1], octets[2], octets[3]
                    ));
                    break;
                }
            }
        }

        // Safety: `ifa_next` is null-terminated per `getifaddrs` contract.
        ifa = unsafe { (*ifa).ifa_next };
    }

    // Always free the list regardless of outcome.
    // Safety: `ifap` was initialised by `getifaddrs` and has not been modified.
    unsafe { libc::freeifaddrs(ifap) };

    result.ok_or_else(|| UdfError::ConnectBack("no non-loopback IPv4 interface found".into()))
}

/// Bridges the input of one group and its emit buffer to the SDK's
/// `UdfContext`, reading the current row via `get` and appending output via
/// `emit`. The dispatcher drives it by input iteration axis:
///
/// - `Multiple` (SET): the UDF advances via `next`, which spans every
///   `MT_NEXT` batch of the group — it fetches the next batch when the current
///   one drains and returns `false` only at the group boundary.
/// - `ExactlyOnce` (SCALAR): the framework drives one `run()` per row via
///   `advance_row`, reading the row through `get`; `next` is banned in this
///   context and returns an error.
///
/// Pulls subsequent batches through a [`BatchFetcher`] the dispatcher installs
/// with `configure_group_input`; the default no-op fetcher yields no further
/// batches, so a bridge built for a single materialised batch (e.g. in tests)
/// iterates that batch and stops.
///
/// Fetches the next `MT_NEXT` batch within a group. `Ok(Some(table))` is a
/// batch, `Ok(None)` the group boundary (the DB answered `MT_DONE`). Shares the
/// dispatcher's single `RefCell<&mut Protocol>` with the emit flusher and the
/// credential fetcher; borrows never overlap because the UDF is single-threaded
/// and the dispatch loop is blocked inside `run()`.
pub type BatchFetcher<'a> =
    Box<dyn FnMut() -> Result<Option<exa_proto::ExascriptTableData>, UdfError> + 'a>;

/// On-demand credential fetcher: given a CONNECTION name, sends MT_IMPORT to the
/// DB and returns the resulting `ConnInfo`. `Fn` (not `FnOnce`) because
/// `connection()` borrows `&self` and may be called repeatedly for different
/// named connections within a single UDF run.
#[cfg(feature = "connect-back")]
pub type ConnRequester<'a> =
    Box<dyn Fn(&str) -> Result<exa_zmq_protocol::ConnInfo, exasol_udf_sdk::error::UdfError> + 'a>;

/// Flushes one pre-built proto table to the DB mid-run. Receives the
/// already-serialised `ExascriptTableData` so the row path and the batch path
/// share the same wire-send logic. Feature-independent: mid-run flushing is not gated on
/// `connect-back`.
pub type EmitFlusher<'a> =
    Box<dyn FnMut(exa_proto::ExascriptTableData) -> Result<(), UdfError> + 'a>;

pub struct HostContextBridge<'a> {
    input: &'a mut InputRowSet,
    emit_buf: &'a mut EmitBuffer,
    input_cols: &'a [ColumnInfo],
    /// Declared EMITS output schema — used by `emit_batch` to choose the target
    /// proto block for each Arrow column. Threaded in by `dispatch::run_group`
    /// alongside `input_cols` because the bridge previously held only the input
    /// columns and `emit_batch` needs the output schema at encoding time.
    output_meta: &'a [ColumnInfo],
    /// Input iteration axis. `Multiple` (SET) drives `next` across batches;
    /// `ExactlyOnce` (SCALAR) presents one row per `run()` via `advance_row` and
    /// bans `next`. Defaults to `Multiple` so a bridge over a single materialised
    /// batch iterates the whole batch.
    input_iter: IterType,
    /// Output iteration axis. `Multiple` (EMITS) admits author `emit`;
    /// `ExactlyOnce` (RETURNS) bans it — the returned value crosses via
    /// `set_return` instead. Defaults to `Multiple` so a bridge constructed
    /// without an explicit output shape accepts `emit`.
    output_iter: IterType,
    /// Pulls the next `MT_NEXT` batch when the current one drains. Defaults to a
    /// no-op yielding no further batches; the dispatcher installs the real
    /// fetcher via `configure_group_input`.
    fetcher: BatchFetcher<'a>,
    started: bool,
    /// Sends one pre-built proto table to the DB when the buffer crosses its byte
    /// threshold, keeping a single batch's output bounded. Invoked from `emit`
    /// and from `push_batch`, both after serialising + clearing the buffer.
    flusher: EmitFlusher<'a>,
    /// Last error captured from a UDF context method. Surfaced through
    /// `RuntimeError::Udf` so the full error appears in the SQL error. A `Cell`
    /// because `connection()` records errors through a shared `&self` borrow.
    last_error: std::cell::Cell<Option<String>>,
    /// Handshake metadata (`exascript_info` identity/origin fields plus the
    /// memory limit) threaded in at construction so the bridge can override the
    /// SDK's defaulted `UdfContext` accessors with the live DB-supplied values.
    handshake: HandshakeMeta,
    #[cfg(feature = "connect-back")]
    conn_requester: ConnRequester<'a>,
}

/// Owned snapshot of the handshake metadata the bridge surfaces to UDF code.
///
/// Bundles the `exascript_info` identity/origin fields and the memory limit so
/// they thread through the bridge constructors as one argument. Strings are
/// owned (not borrowed) because the corresponding `UdfContext` accessors
/// return owned `String`/`Option<String>` across the `.so` vtable boundary.
/// Built from a `&UdfMeta` via `From`; `Default` yields the all-neutral value
/// tests use.
///
/// The iteration axes are deliberately not here: `HostContextBridge` already
/// owns them natively (set by `configure_group_input`), so a copy here would
/// sit unused; `SingleCallContext` holds its own axis fields for the same
/// reason instead of reading them off this struct.
#[derive(Debug, Clone, Default)]
pub struct HandshakeMeta {
    pub session_id: u64,
    pub statement_id: u32,
    pub node_id: u32,
    pub node_count: u32,
    pub vm_id: u64,
    pub memory_limit: u64,
    pub database_name: String,
    pub database_version: String,
    pub script_name: String,
    pub script_schema: String,
    pub current_user: Option<String>,
    pub current_schema: Option<String>,
    pub scope_user: Option<String>,
}

impl From<&exa_zmq_protocol::UdfMeta> for HandshakeMeta {
    fn from(meta: &exa_zmq_protocol::UdfMeta) -> Self {
        HandshakeMeta {
            session_id: meta.session_id(),
            statement_id: meta.statement_id(),
            node_id: meta.node_id(),
            node_count: meta.node_count(),
            vm_id: meta.vm_id(),
            memory_limit: meta.maximal_memory_limit,
            database_name: meta.database_name.clone(),
            database_version: meta.database_version.clone(),
            script_name: meta.script_name.clone(),
            script_schema: meta.script_schema.clone(),
            current_user: meta.current_user.clone(),
            current_schema: meta.current_schema.clone(),
            scope_user: meta.scope_user.clone(),
        }
    }
}

impl<'a> HostContextBridge<'a> {
    pub fn new(
        input: &'a mut InputRowSet,
        emit_buf: &'a mut EmitBuffer,
        input_cols: &'a [ColumnInfo],
        output_meta: &'a [ColumnInfo],
        flusher: EmitFlusher<'a>,
        handshake: HandshakeMeta,
        #[cfg(feature = "connect-back")] conn_requester: ConnRequester<'a>,
    ) -> Self {
        HostContextBridge {
            input,
            emit_buf,
            input_cols,
            output_meta,
            input_iter: IterType::Multiple,
            output_iter: IterType::Multiple,
            fetcher: Box::new(|| Ok(None)),
            started: false,
            flusher,
            last_error: std::cell::Cell::new(None),
            handshake,
            #[cfg(feature = "connect-back")]
            conn_requester,
        }
    }

    /// Install the group's iteration axes and batch fetcher. Called once by the
    /// dispatcher after construction; the fetcher spans `MT_NEXT` batches within
    /// the group for both the scalar per-row loop and set `next`. The output axis
    /// governs whether author `emit` is admitted (EMITS) or banned in favour of
    /// `set_return` (RETURNS).
    pub fn configure_group_input(
        &mut self,
        input_iter: IterType,
        output_iter: IterType,
        fetcher: BatchFetcher<'a>,
    ) {
        self.input_iter = input_iter;
        self.output_iter = output_iter;
        self.fetcher = fetcher;
    }

    /// Advance the framework cursor to the next scalar row, fetching the next
    /// `MT_NEXT` batch when the current one drains. Returns `false` at the group
    /// boundary.
    pub fn advance_row(&mut self) -> Result<bool, UdfError> {
        if self.input.advance() {
            Ok(true)
        } else {
            self.refill()
        }
    }

    /// Fetch batches until a non-empty one loads (presenting its row 0) or the
    /// group ends. Zero-row batches are skipped. Returns `false` only at the
    /// group boundary.
    fn refill(&mut self) -> Result<bool, UdfError> {
        loop {
            match (self.fetcher)()? {
                Some(table) => {
                    *self.input = InputRowSet::from_proto(&table, self.input_cols);
                    if !self.input.is_empty() {
                        return Ok(true);
                    }
                }
                None => return Ok(false),
            }
        }
    }

    /// Append one output row to the group-scoped buffer, flushing an `MT_EMIT`
    /// when the running byte estimate crosses the threshold. Shared by `emit`
    /// (EMITS output) and `set_return` (RETURNS output) so both honour the same
    /// validation, buffering and flush contract; the trailing rows are flushed by the
    /// dispatcher before the group's `MT_DONE`.
    fn push_output_row(&mut self, row: Vec<Value>) -> Result<(), UdfError> {
        let row_cost = check_output_row(&row, self.output_meta)?;
        self.emit_buf.push_costed(
            row,
            self.input.current_row_number(),
            self.output_meta,
            row_cost,
        );
        if self.emit_buf.should_flush() {
            self.emit_buf.record_flush_telemetry();
            let table = self.emit_buf.take_proto();
            (self.flusher)(table)?;
        }
        Ok(())
    }

    /// Take the last error message captured from a UDF context method.
    pub fn take_last_error(&mut self) -> Option<String> {
        self.last_error.take()
    }

    /// Record an error message captured from a UDF context method. Available on
    /// a shared borrow because `connection()` is a `&self` method.
    #[cfg(feature = "connect-back")]
    fn record_error(&self, message: String) {
        self.last_error.set(Some(message));
    }

    /// Inject a credential fetcher so the bridge can be exercised without a live
    /// database. The supplied closure stands in for the on-demand MT_IMPORT
    /// exchange. Intended for tests.
    #[cfg(feature = "connect-back")]
    #[doc(hidden)]
    pub fn with_connection(
        input: &'a mut InputRowSet,
        emit_buf: &'a mut EmitBuffer,
        input_cols: &'a [ColumnInfo],
        output_meta: &'a [ColumnInfo],
        flusher: EmitFlusher<'a>,
        handshake: HandshakeMeta,
        conn_requester: ConnRequester<'a>,
    ) -> Self {
        HostContextBridge {
            input,
            emit_buf,
            input_cols,
            output_meta,
            input_iter: IterType::Multiple,
            output_iter: IterType::Multiple,
            fetcher: Box::new(|| Ok(None)),
            started: false,
            flusher,
            last_error: std::cell::Cell::new(None),
            handshake,
            conn_requester,
        }
    }
}

/// Resolve a CONNECTION name to a [`ConnectionObject`] via the on-demand
/// credential fetcher. Shared by both context bridges.
#[cfg(feature = "connect-back")]
fn request_connection(
    requester: &ConnRequester,
    name: &str,
) -> Result<exasol_udf_sdk::connect_back::ConnectionObject, UdfError> {
    requester(name).map(|ci| exasol_udf_sdk::connect_back::ConnectionObject {
        kind: ci.kind,
        address: ci.address,
        user: ci.user,
        password: ci.password,
    })
}

/// Open a self-connection back to the DB from a resolved [`ConnectionObject`].
/// Shared by both context bridges.
#[cfg(feature = "connect-back")]
fn open_connect_back(
    conn: &exasol_udf_sdk::connect_back::ConnectionObject,
) -> Result<Box<dyn exasol_udf_sdk::connect_back::ExaConnection>, UdfError> {
    let info = exa_zmq_protocol::ConnInfo {
        kind: conn.kind.clone(),
        address: conn.address.clone(),
        user: conn.user.clone(),
        password: conn.password.clone(),
    };
    crate::connect_back::open_connection(&info)
        .map(|c| Box::new(c) as Box<dyn exasol_udf_sdk::connect_back::ExaConnection>)
}

/// The `UdfContext` handshake-metadata getters. `HostContextBridge` and
/// `SingleCallContext` both forward them to their `handshake` field identically.
macro_rules! delegate_handshake_meta {
    () => {
        fn memory_limit(&self) -> u64 {
            self.handshake.memory_limit
        }

        fn session_id(&self) -> u64 {
            self.handshake.session_id
        }

        fn statement_id(&self) -> u32 {
            self.handshake.statement_id
        }

        fn node_id(&self) -> u32 {
            self.handshake.node_id
        }

        fn node_count(&self) -> u32 {
            self.handshake.node_count
        }

        fn vm_id(&self) -> u64 {
            self.handshake.vm_id
        }

        fn database_name(&self) -> String {
            self.handshake.database_name.clone()
        }

        fn database_version(&self) -> String {
            self.handshake.database_version.clone()
        }

        fn script_name(&self) -> String {
            self.handshake.script_name.clone()
        }

        fn script_schema(&self) -> String {
            self.handshake.script_schema.clone()
        }

        fn current_user(&self) -> Option<String> {
            self.handshake.current_user.clone()
        }

        fn current_schema(&self) -> Option<String> {
            self.handshake.current_schema.clone()
        }

        fn scope_user(&self) -> Option<String> {
            self.handshake.scope_user.clone()
        }

        fn debug_level(&self) -> tracing::Level {
            current_debug_level()
        }
    };
}

/// The `connect-back` `UdfContext` hooks. Both contexts forward them to the
/// shared machinery identically, each recording the error on failure.
macro_rules! delegate_connect_back_hooks {
    () => {
        #[cfg(feature = "connect-back")]
        fn cluster_ip(&self) -> Result<String, UdfError> {
            let result = first_nonloopback_ipv4();
            if let Err(ref e) = result {
                self.record_error(e.to_string());
            }
            result
        }

        #[cfg(feature = "connect-back")]
        fn connection(
            &self,
            name: &str,
        ) -> Result<exasol_udf_sdk::connect_back::ConnectionObject, UdfError> {
            let result = request_connection(&self.conn_requester, name);
            if let Err(ref e) = result {
                self.record_error(e.to_string());
            }
            result
        }

        #[cfg(feature = "connect-back")]
        fn connect_back(
            &mut self,
            conn: &exasol_udf_sdk::connect_back::ConnectionObject,
        ) -> Result<Box<dyn exasol_udf_sdk::connect_back::ExaConnection>, UdfError> {
            let result = open_connect_back(conn);
            if let Err(ref e) = result {
                self.record_error(e.to_string());
            }
            result
        }
    };
}

impl UdfContext for HostContextBridge<'_> {
    fn input_column_count(&self) -> usize {
        self.input_cols.len()
    }

    fn input_column(&self, idx: usize) -> Result<&ColumnInfo, UdfError> {
        self.input_cols
            .get(idx)
            .ok_or_else(|| UdfError::Type(format!("input column {idx} out of range")))
    }

    fn output_column_count(&self) -> usize {
        self.output_meta.len()
    }

    fn output_column(&self, idx: usize) -> Result<&ColumnInfo, UdfError> {
        self.output_meta
            .get(idx)
            .ok_or_else(|| UdfError::Type(format!("output column {idx} out of range")))
    }

    delegate_handshake_meta!();

    fn get(&self, col: usize) -> Result<&Value, UdfError> {
        self.input
            .current_row()
            .get(col)
            .ok_or_else(|| UdfError::Type(format!("column {col} out of range")))
    }

    fn emit(&mut self, values: Vec<Value>) -> Result<(), UdfError> {
        if self.output_iter == IterType::ExactlyOnce {
            return Err(UdfError::User(
                "emit() is not allowed in RETURNS output context; return the value instead".into(),
            ));
        }
        self.push_output_row(values)
    }

    fn set_return(&mut self, value: Option<Value>) -> Result<(), UdfError> {
        self.push_output_row(vec![value.unwrap_or(Value::Null)])
    }

    #[cfg(feature = "emit-arrow")]
    fn emit_record_batch_ipc(&mut self, ipc: &[u8]) -> Result<(), UdfError> {
        if self.output_iter == IterType::ExactlyOnce {
            return Err(UdfError::User(
                "emit() is not allowed in RETURNS output context; return the value instead".into(),
            ));
        }
        // Resolve the disjoint borrow: take references to fields we need
        // separately so the borrow checker sees them as independent borrows.
        let row_number = self.input.current_row_number();
        let emit_buf = &mut *self.emit_buf;
        let flusher = &mut self.flusher;
        let meta = self.output_meta;
        // Deserialise into a host-owned RecordBatch (single arrow copy on this
        // side of the .so boundary), then replay the existing push_batch path.
        let reader = arrow::ipc::reader::StreamReader::try_new(std::io::Cursor::new(ipc), None)
            .map_err(|e| UdfError::Type(format!("emit_batch: IPC reader init: {e}")))?;
        for batch in reader {
            let batch = batch.map_err(|e| UdfError::Type(format!("emit_batch: IPC read: {e}")))?;
            emit_buf.push_batch(&batch, meta, row_number, &mut |table| (flusher)(table))?;
        }
        Ok(())
    }

    fn next(&mut self) -> Result<bool, UdfError> {
        match self.input_iter {
            // SCALAR: the framework drives one `run()` per input row via
            // `advance_row`, so `next()` has no meaning here. Reject it, matching
            // the reference containers' ban on `next()` in scalar context.
            IterType::ExactlyOnce => Err(UdfError::User(
                "next() is not allowed in scalar context".into(),
            )),
            // SET: span every `MT_NEXT` batch of the group. The first call
            // positions on row 0 of the current batch; subsequent calls advance,
            // fetching the next batch when the current drains. `false` only at
            // the group boundary.
            IterType::Multiple => {
                if !self.started {
                    self.started = true;
                    if !self.input.is_empty() {
                        return Ok(true);
                    }
                    return self.refill();
                }
                if self.input.advance() {
                    return Ok(true);
                }
                self.refill()
            }
        }
    }

    fn rows_in_group(&self) -> u64 {
        self.input.rows_in_group()
    }

    fn input_type(&self) -> Option<InputType> {
        Some(input_type_of(self.input_iter))
    }

    fn output_type(&self) -> Option<OutputType> {
        Some(output_type_of(self.output_iter))
    }

    delegate_connect_back_hooks!();
}

/// A `UdfContext` for single-call mode (e.g. the virtual-schema adapter call).
///
/// Single-call hooks receive no input rows and emit no output rows: the DB
/// exchanges one JSON request for one JSON response. The data methods therefore
/// return [`UdfError::Unimplemented`] — only credential resolution
/// (`connection`) and self-connections (`connect_back`) are meaningful, and
/// those reuse the same on-demand MT_IMPORT machinery as the data-UDF bridge.
pub struct SingleCallContext<'a> {
    /// Last error captured from a context method, surfaced through
    /// `RuntimeError::Udf`. A `Cell` because `connection()` borrows `&self`.
    last_error: std::cell::Cell<Option<String>>,
    /// Handshake metadata (`exascript_info` identity/origin fields plus the
    /// memory limit) threaded in at construction so the single-call context can
    /// override the SDK's defaulted `UdfContext` accessors with the live
    /// DB-supplied values, giving parity with `HostContextBridge`.
    handshake: HandshakeMeta,
    /// Declared iteration axes, read directly (not via `handshake`) for the
    /// same reason `HostContextBridge` keeps its own copy: single-call mode
    /// has no group/batch state to derive them from otherwise.
    input_iter: IterType,
    output_iter: IterType,
    #[cfg(feature = "connect-back")]
    conn_requester: ConnRequester<'a>,
    /// Anchors the `'a` lifetime when connect-back is disabled (the requester is
    /// the only `'a` user otherwise).
    #[cfg(not(feature = "connect-back"))]
    _marker: std::marker::PhantomData<&'a ()>,
}

impl<'a> SingleCallContext<'a> {
    pub fn new(
        handshake: HandshakeMeta,
        input_iter: IterType,
        output_iter: IterType,
        #[cfg(feature = "connect-back")] conn_requester: ConnRequester<'a>,
    ) -> Self {
        SingleCallContext {
            last_error: std::cell::Cell::new(None),
            handshake,
            input_iter,
            output_iter,
            #[cfg(feature = "connect-back")]
            conn_requester,
            #[cfg(not(feature = "connect-back"))]
            _marker: std::marker::PhantomData,
        }
    }

    /// Take the last error message captured from a context method.
    pub fn take_last_error(&mut self) -> Option<String> {
        self.last_error.take()
    }

    #[cfg(feature = "connect-back")]
    fn record_error(&self, message: String) {
        self.last_error.set(Some(message));
    }
}

impl UdfContext for SingleCallContext<'_> {
    fn input_column_count(&self) -> usize {
        0
    }

    delegate_handshake_meta!();

    fn get(&self, _col: usize) -> Result<&Value, UdfError> {
        Err(UdfError::Unimplemented(
            "single-call mode has no input columns".into(),
        ))
    }

    fn emit(&mut self, _values: Vec<Value>) -> Result<(), UdfError> {
        Err(UdfError::Unimplemented(
            "single-call mode does not emit rows".into(),
        ))
    }

    fn next(&mut self) -> Result<bool, UdfError> {
        Err(UdfError::Unimplemented(
            "single-call mode has no input rows".into(),
        ))
    }

    fn input_type(&self) -> Option<InputType> {
        Some(input_type_of(self.input_iter))
    }

    fn output_type(&self) -> Option<OutputType> {
        Some(output_type_of(self.output_iter))
    }

    delegate_connect_back_hooks!();
}

#[cfg(test)]
#[path = "rowset_tests.rs"]
mod tests;
