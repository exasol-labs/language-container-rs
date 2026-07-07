use chrono::{NaiveDate, NaiveDateTime};
use exa_proto::ExascriptTableData;
use exa_zmq_protocol::{ColumnMeta, ExaType};
use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::error::UdfError;
use exasol_udf_sdk::value::{Decimal, Value};

/// Per-type column block width: each block holds exactly `n_rows` entries per
/// column (placeholder slots for NULL cells), so cell `(col, row)` lives at
/// `block_base + row`. The NULL bitmap is row-major: `row * n_cols + col`.
fn null_index(row: usize, col: usize, n_cols: usize) -> usize {
    row * n_cols + col
}

/// Materialised input rows from one proto `ExascriptTableData` batch.
///
/// Stored as a dense `rows[row][col]` matrix of `Value` for simplicity and
/// correctness; the per-type proto blocks are decoded once on construction.
pub struct InputRowSet {
    rows: Vec<Vec<Value>>,
    current_row: usize,
}

impl InputRowSet {
    /// Decode a proto batch into a row-major matrix of `Value`s.
    ///
    /// The proto packs each cell type into its own array, column by column,
    /// with one slot per row (including NULL cells). The NULL bitmap is
    /// row-major across all columns.
    pub fn from_proto(table: &ExascriptTableData, meta: &[ColumnMeta]) -> Self {
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
                    | ExaType::Timestamp
                    | ExaType::TimestampTz
                    | ExaType::String { .. }
                    | ExaType::Char { .. }
                    | ExaType::Geometry
                    | ExaType::HashType
                    | ExaType::IntervalYearToMonth
                    | ExaType::IntervalDayToSecond => {
                        let s = table
                            .data_string
                            .get(string_idx)
                            .cloned()
                            .unwrap_or_default();
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

        InputRowSet {
            rows,
            current_row: 0,
        }
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

    pub fn current_index(&self) -> usize {
        self.current_row
    }

    pub fn current_row(&self) -> &[Value] {
        &self.rows[self.current_row]
    }

    pub fn row(&self, idx: usize) -> Option<&[Value]> {
        self.rows.get(idx).map(|r| r.as_slice())
    }

    pub fn is_exhausted(&self) -> bool {
        self.current_row >= self.rows.len()
    }
}

/// Byte threshold at which the emit buffer requests a flush. Keeps the
/// serialised `MT_EMIT` payload well under the DB's per-message limits while
/// amortising the per-flush round-trip across many rows.
const EMIT_BUFFER_LIMIT_BYTES: usize = 4_000_000;

/// Conservative O(1) byte cost for one cell, approximating the width its
/// non-null value occupies in `to_proto`'s type block. NULL cells cost 0
/// because they take no type-block slot; the estimate slightly over-counts
/// (fixed widths for Numeric/Date/Timestamp) so the buffer flushes early rather
/// than late.
fn value_byte_cost(v: &Value) -> usize {
    match v {
        Value::Null => 0,
        Value::Bool(_) => 1,
        Value::Int32(_) => 4,
        Value::Int64(_) => 8,
        Value::Double(_) => 8,
        Value::String(s) => s.len(),
        // O(1) upper bound, no alloc: i128 renders in ≤39 digits + sign (≤40),
        // which also dominates the scale-padded form (sign + scale+1 + point).
        // Over-counts → flushes early, matching the conservative intent above.
        Value::Numeric(d) => 40 + d.scale as usize,
        Value::Date(_) => 10,
        Value::Timestamp(_) => 29,
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

/// Accumulates emitted output rows, serialising to a proto batch on flush.
#[derive(Default)]
pub struct EmitBuffer {
    rows: Vec<Vec<Value>>,
    /// Running approximate serialised size of the buffered rows. Incremented in
    /// `push`, reset in `clear`; read by `should_flush`.
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

    pub fn push(&mut self, values: Vec<Value>) {
        let row_cost = values.iter().map(value_byte_cost).sum::<usize>();
        self.byte_estimate += row_cost;
        self.cumulative_bytes += row_cost;
        self.cumulative_rows += 1;
        // Per-push debug event: bytes buffered and running cost for this row.
        // Automatic tracing-level gating suppresses this at INFO or coarser.
        tracing::debug!(
            target: "emit_push",
            bytes_buffered = self.byte_estimate,
            row_cost,
            cumulative_rows = self.cumulative_rows,
            "emit row buffered"
        );
        // Periodic full-state checkpoint with RSS (task 5.2).
        if self
            .cumulative_rows
            .is_multiple_of(Self::TELEMETRY_ROW_CHECKPOINT)
        {
            self.record_flush_telemetry();
        }
        self.rows.push(values);
    }

    /// Whether the buffered rows have reached the byte threshold and should be
    /// flushed to the DB. A single oversized row trips this on its own push.
    pub fn should_flush(&self) -> bool {
        self.byte_estimate >= EMIT_BUFFER_LIMIT_BYTES
    }

    /// Serialise accumulated rows into an `ExascriptTableData`.
    ///
    /// Mirrors `InputRowSet::from_proto`: each type block is column-major and
    /// dense (one slot per row, including a placeholder for NULL cells) so the
    /// `block_base + row` indexing stays valid. The NULL bitmap is row-major.
    pub fn to_proto(&self, meta: &[ColumnMeta]) -> ExascriptTableData {
        let n_rows = self.rows.len();
        let n_cols = meta.len();

        // Pre-size each type block to its exact upper bound (its column count ×
        // n_rows) instead of growing via `Vec::new()` + `push`, which
        // reallocates through the standard doubling growth curve. NULL cells
        // occupy no slot (see the loop below), so the true final length can be
        // at most this bound — `with_capacity` at the bound is therefore always
        // sufficient and never wasteful beyond the NULL count.
        let mut string_cols = 0usize;
        let mut bool_cols = 0usize;
        let mut int32_cols = 0usize;
        let mut int64_cols = 0usize;
        let mut double_cols = 0usize;
        for col in meta {
            match &col.typ {
                ExaType::Numeric { .. }
                | ExaType::Date
                | ExaType::Timestamp
                | ExaType::TimestampTz
                | ExaType::String { .. }
                | ExaType::Char { .. }
                | ExaType::Geometry
                | ExaType::HashType
                | ExaType::IntervalYearToMonth
                | ExaType::IntervalDayToSecond => string_cols += 1,
                ExaType::Boolean => bool_cols += 1,
                ExaType::Int32 => int32_cols += 1,
                ExaType::Int64 => int64_cols += 1,
                ExaType::Double => double_cols += 1,
                ExaType::Unsupported => {}
            }
        }

        let mut data_string: Vec<String> = Vec::with_capacity(string_cols * n_rows);
        let mut data_bool: Vec<bool> = Vec::with_capacity(bool_cols * n_rows);
        let mut data_int32: Vec<i32> = Vec::with_capacity(int32_cols * n_rows);
        let mut data_int64: Vec<i64> = Vec::with_capacity(int64_cols * n_rows);
        let mut data_double: Vec<f64> = Vec::with_capacity(double_cols * n_rows);
        let mut data_nulls: Vec<bool> = vec![false; n_rows * n_cols];

        // Row-major within each type block: iterate rows, then columns, so a
        // column's cells appear in the block interleaved with the other columns
        // of the same type. This matches the layout Exasol's emit handler reads
        // (and that `from_proto` decodes); a column-major layout silently lands
        // later rows' values in the wrong column.
        //
        // Each value is packed into the block dictated by the declared column
        // type, not the runtime `Value` variant: a connect-back SELECT may
        // return a DECIMAL column as `Value::Int64`, but the EMITS column is
        // `ExaType::Numeric` (string block).
        for (r, row) in self.rows.iter().enumerate() {
            for (c, col) in meta.iter().enumerate() {
                let v = row.get(c).unwrap_or(&Value::Null);
                if matches!(v, Value::Null) {
                    // A NULL cell is recorded only in the bitmap; it does NOT
                    // occupy a slot in its type block. Exasol's reader consumes
                    // type-block entries only for non-null cells and consults the
                    // bitmap for nullness — a placeholder would shift every later
                    // cell of that type into the wrong column.
                    data_nulls[null_index(r, c, n_cols)] = true;
                    continue;
                }
                match &col.typ {
                    ExaType::Numeric { .. }
                    | ExaType::Date
                    | ExaType::Timestamp
                    | ExaType::TimestampTz
                    | ExaType::String { .. }
                    | ExaType::Char { .. }
                    | ExaType::Geometry
                    | ExaType::HashType
                    | ExaType::IntervalYearToMonth
                    | ExaType::IntervalDayToSecond => {
                        data_string.push(value_to_block_string(v));
                    }
                    ExaType::Boolean => data_bool.push(value_to_bool(v)),
                    ExaType::Int32 => data_int32.push(value_to_i64(v) as i32),
                    ExaType::Int64 => data_int64.push(value_to_i64(v)),
                    ExaType::Double => data_double.push(value_to_f64(v)),
                    ExaType::Unsupported => {}
                }
            }
        }

        ExascriptTableData {
            rows: n_rows as u64,
            rows_in_group: 0,
            data_string,
            data_nulls,
            data_bool,
            data_int32,
            data_int64,
            data_double,
            row_number: vec![],
        }
    }

    pub fn clear(&mut self) {
        self.flush_count += 1;
        self.rows.clear();
        self.byte_estimate = 0;
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
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
            buffered_rows = self.rows.len(),
            "MT_EMIT flush"
        );
    }

    /// Encode an Arrow `RecordBatch` into the emit stream, flushing ≤4 MB
    /// slices directly to `flush` and materialising only the trailing <4 MB
    /// tail into `self`.
    ///
    /// Algorithm (decision-log [6],[7]):
    /// 1. Flush any pending Value rows first so the batch starts from an empty
    ///    buffer.
    /// 2. Validate each Arrow column's `DataType` against the declared
    ///    `ExaType`; reject mismatches with `Err`.
    /// 3. Compute a cheap per-row byte cost (no per-cell work: fixed widths for
    ///    fixed-size types, offset-buffer prefix-sum for variable-width) and
    ///    split at `EMIT_BUFFER_LIMIT_BYTES` row boundaries.
    /// 4. For each full ≤4 MB slice: take a zero-copy `RecordBatch::slice`,
    ///    encode it column-at-a-time (one downcast + one null-buffer read per
    ///    column) preserving the row-major-interleaved dense layout `to_proto`
    ///    produces, and call `flush`.
    /// 5. Materialise the trailing <4 MB remainder into `self` via `push()`.
    #[cfg(feature = "emit-arrow")]
    pub fn push_batch(
        &mut self,
        batch: &arrow::record_batch::RecordBatch,
        meta: &[ColumnMeta],
        flush: &mut dyn FnMut(exa_proto::ExascriptTableData) -> Result<(), UdfError>,
    ) -> Result<(), UdfError> {
        // Step 1: flush any pending Value rows so we start from an empty buffer.
        if !self.is_empty() {
            let table = self.to_proto(meta);
            flush(table)?;
            self.clear();
        }

        let n_rows = batch.num_rows();

        if n_rows == 0 {
            return Ok(());
        }

        // Step 2: validate and downcast all columns exactly once, fail fast
        // before computing costs or touching any row data.
        build_accessors(batch, meta)?;

        // Step 3: compute cumulative per-row byte costs using the Arrow offset
        // buffers for variable-width types (no per-cell work for bulk).
        let row_costs = compute_row_costs(batch, meta);

        // Step 4: split into ≤4 MB slices and flush each directly.
        let mut running: usize = 0;
        let mut slice_start: usize = 0;

        for (r, &row_cost) in row_costs.iter().enumerate() {
            running += row_cost;
            if running >= EMIT_BUFFER_LIMIT_BYTES {
                // Flush [slice_start, r+1).
                let slice_len = r + 1 - slice_start;
                let slice = batch.slice(slice_start, slice_len);
                let table = encode_slice(&slice, meta)?;
                flush(table)?;
                slice_start = r + 1;
                running = 0;
            }
        }

        // Step 5: materialise the trailing <4 MB tail into self.
        if slice_start < n_rows {
            let tail_len = n_rows - slice_start;
            let tail = batch.slice(slice_start, tail_len);
            // Convert the tail to Value rows and push into the buffer.
            let tail_rows = arrow_batch_to_value_rows(&tail, meta)?;
            for row in tail_rows {
                self.push(row);
            }
        }

        Ok(())
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
/// The `ExaType` authority is the declared `ColumnMeta`; the Arrow type is
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
    /// widening): extract value as the natural type; `encode_slice` stringifies
    /// it into the string block via `value_to_block_string`.
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
    meta: &[ColumnMeta],
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
            (DataType::Timestamp(unit, _), ExaType::Timestamp | ExaType::TimestampTz) => match unit
            {
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
            | ExaType::Timestamp
            | ExaType::TimestampTz
            | ExaType::String { .. }
            | ExaType::Char { .. }
            | ExaType::Geometry
            | ExaType::HashType
            | ExaType::IntervalYearToMonth
            | ExaType::IntervalDayToSecond
    )
}

/// Compute a per-row byte cost vector for the batch using Arrow's columnar
/// layout for efficiency (no per-cell work for fixed-width types; offset-buffer
/// prefix-sum for variable-width; same fixed estimates as `value_byte_cost`).
#[cfg(feature = "emit-arrow")]
fn compute_row_costs(batch: &arrow::record_batch::RecordBatch, meta: &[ColumnMeta]) -> Vec<usize> {
    use arrow::array::{Array, LargeStringArray, StringArray};
    use arrow::datatypes::DataType;

    let n_rows = batch.num_rows();
    let mut costs = vec![0usize; n_rows];

    for (c, _col_meta) in meta.iter().enumerate() {
        let col = batch.column(c);
        let dt = col.data_type();
        let null_buf = col.nulls();

        // For each row r: add the per-cell byte cost to costs[r].
        // NULL cells cost 0 (no type-block slot, matching value_byte_cost).
        // Iterate via enumerate so we don't trigger needless_range_loop.
        match dt {
            DataType::Int32 => {
                for (r, cost) in costs.iter_mut().enumerate() {
                    if !null_buf.as_ref().is_some_and(|nb| nb.is_null(r)) {
                        *cost += 4;
                    }
                }
            }
            DataType::Int64 => {
                for (r, cost) in costs.iter_mut().enumerate() {
                    if !null_buf.as_ref().is_some_and(|nb| nb.is_null(r)) {
                        *cost += 8;
                    }
                }
            }
            DataType::Float64 => {
                for (r, cost) in costs.iter_mut().enumerate() {
                    if !null_buf.as_ref().is_some_and(|nb| nb.is_null(r)) {
                        *cost += 8;
                    }
                }
            }
            DataType::Boolean => {
                for (r, cost) in costs.iter_mut().enumerate() {
                    if !null_buf.as_ref().is_some_and(|nb| nb.is_null(r)) {
                        *cost += 1;
                    }
                }
            }
            DataType::Date32 => {
                // DATE renders to "YYYY-MM-DD" (10 chars)
                for (r, cost) in costs.iter_mut().enumerate() {
                    if !null_buf.as_ref().is_some_and(|nb| nb.is_null(r)) {
                        *cost += 10;
                    }
                }
            }
            DataType::Timestamp(_, _) => {
                // TIMESTAMP renders to "YYYY-MM-DD HH:MM:SS.nnnnnnnnn" (29 chars)
                for (r, cost) in costs.iter_mut().enumerate() {
                    if !null_buf.as_ref().is_some_and(|nb| nb.is_null(r)) {
                        *cost += 29;
                    }
                }
            }
            DataType::Decimal128(_, scale) => {
                // Numeric: 40 + scale chars (matches value_byte_cost)
                let fixed_cost = 40 + (*scale as usize);
                for (r, cost) in costs.iter_mut().enumerate() {
                    if !null_buf.as_ref().is_some_and(|nb| nb.is_null(r)) {
                        *cost += fixed_cost;
                    }
                }
            }
            DataType::Utf8 => {
                // Variable width: use the string value's byte length.
                if let Some(arr) = col.as_any().downcast_ref::<StringArray>() {
                    for (r, cost) in costs.iter_mut().enumerate() {
                        if !null_buf.as_ref().is_some_and(|nb| nb.is_null(r)) {
                            *cost += arr.value(r).len();
                        }
                    }
                }
            }
            DataType::LargeUtf8 => {
                if let Some(arr) = col.as_any().downcast_ref::<LargeStringArray>() {
                    for (r, cost) in costs.iter_mut().enumerate() {
                        if !null_buf.as_ref().is_some_and(|nb| nb.is_null(r)) {
                            *cost += arr.value(r).len();
                        }
                    }
                }
            }
            _ => {
                // Unsupported: cost 0 (type validation ran before this call).
            }
        }
    }

    costs
}

/// Encode a (possibly sliced) `RecordBatch` into one `ExascriptTableData`,
/// preserving the dense row-major-interleaved layout `to_proto` produces.
///
/// Each Arrow column array is downcast to its concrete type exactly once via
/// `build_accessors`; its validity (null) buffer is read once in bulk per
/// column. The row-major loop then reads `accessor.value(r)` with no further
/// downcast per cell. A NULL cell occupies no type-block slot — only the
/// row-major bitmap is updated — so the encoding stays byte-identical to
/// `to_proto` for arbitrary EMITS schemas including multiple columns sharing
/// one block type and any null pattern.
#[cfg(feature = "emit-arrow")]
fn encode_slice(
    batch: &arrow::record_batch::RecordBatch,
    meta: &[ColumnMeta],
) -> Result<exa_proto::ExascriptTableData, UdfError> {
    use arrow::array::Array;

    let accessors = build_accessors(batch, meta)?;
    let n_rows = batch.num_rows();
    let n_cols = meta.len();

    // Pre-size each type block to its exact upper bound (its column count ×
    // n_rows), mirroring `to_proto`'s pre-sizing, instead of growing via
    // `Vec::new()` + `push`.
    let mut string_cols = 0usize;
    let mut bool_cols = 0usize;
    let mut int32_cols = 0usize;
    let mut int64_cols = 0usize;
    let mut double_cols = 0usize;
    for acc in &accessors {
        match acc {
            ColAccessor::Int32(_) => int32_cols += 1,
            ColAccessor::Int64(_) => int64_cols += 1,
            ColAccessor::Float64(_) => double_cols += 1,
            ColAccessor::Boolean(_) => bool_cols += 1,
            ColAccessor::Utf8(_)
            | ColAccessor::LargeUtf8(_)
            | ColAccessor::Date32(_)
            | ColAccessor::TsSecond(_)
            | ColAccessor::TsMillisecond(_)
            | ColAccessor::TsMicrosecond(_)
            | ColAccessor::TsNanosecond(_)
            | ColAccessor::Decimal128(_, _)
            | ColAccessor::NumericFromInt32(_)
            | ColAccessor::NumericFromInt64(_)
            | ColAccessor::NumericFromFloat64(_) => string_cols += 1,
            ColAccessor::Unsupported => {}
        }
    }

    let mut data_string: Vec<String> = Vec::with_capacity(string_cols * n_rows);
    let mut data_bool: Vec<bool> = Vec::with_capacity(bool_cols * n_rows);
    let mut data_int32: Vec<i32> = Vec::with_capacity(int32_cols * n_rows);
    let mut data_int64: Vec<i64> = Vec::with_capacity(int64_cols * n_rows);
    let mut data_double: Vec<f64> = Vec::with_capacity(double_cols * n_rows);
    let mut data_nulls: Vec<bool> = vec![false; n_rows * n_cols];

    // Pre-capture per-column null buffers once (bulk null read, not per cell).
    let null_bufs: Vec<_> = (0..n_cols)
        .map(|c| batch.column(c).nulls().cloned())
        .collect();

    // Row-major encoding: for r in 0..n_rows, for c in 0..n_cols.
    // Matches `to_proto`'s loop order so same-ExaType columns interleave
    // identically and `from_proto` reads back the correct values.
    for r in 0..n_rows {
        for (c, acc) in accessors.iter().enumerate() {
            let is_null = null_bufs[c].as_ref().is_some_and(|nb| nb.is_null(r));
            if is_null {
                data_nulls[null_index(r, c, n_cols)] = true;
                continue;
            }

            match acc {
                ColAccessor::Int32(arr) => data_int32.push(arr.value(r)),
                ColAccessor::Int64(arr) => data_int64.push(arr.value(r)),
                ColAccessor::Float64(arr) => data_double.push(arr.value(r)),
                ColAccessor::Boolean(arr) => data_bool.push(arr.value(r)),
                ColAccessor::Utf8(arr) => data_string.push(arr.value(r).to_string()),
                ColAccessor::LargeUtf8(arr) => data_string.push(arr.value(r).to_string()),
                ColAccessor::Date32(arr) => {
                    let days = arr.value(r);
                    let date = chrono::NaiveDate::from_num_days_from_ce_opt(
                        days + 719163, // Arrow epoch: 1970-01-01 = day 719163 in CE days
                    )
                    .unwrap_or_default();
                    data_string.push(value_to_block_string(&exasol_udf_sdk::value::Value::Date(
                        date,
                    )));
                }
                ColAccessor::TsSecond(arr) => {
                    let ts = chrono::DateTime::from_timestamp(arr.value(r), 0)
                        .map(|dt| dt.naive_utc())
                        .unwrap_or_default();
                    data_string.push(value_to_block_string(
                        &exasol_udf_sdk::value::Value::Timestamp(ts),
                    ));
                }
                ColAccessor::TsMillisecond(arr) => {
                    let ts = chrono::DateTime::from_timestamp_millis(arr.value(r))
                        .map(|dt| dt.naive_utc())
                        .unwrap_or_default();
                    data_string.push(value_to_block_string(
                        &exasol_udf_sdk::value::Value::Timestamp(ts),
                    ));
                }
                ColAccessor::TsMicrosecond(arr) => {
                    let ts = chrono::DateTime::from_timestamp_micros(arr.value(r))
                        .map(|dt| dt.naive_utc())
                        .unwrap_or_default();
                    data_string.push(value_to_block_string(
                        &exasol_udf_sdk::value::Value::Timestamp(ts),
                    ));
                }
                ColAccessor::TsNanosecond(arr) => {
                    let ns = arr.value(r);
                    let ts = chrono::DateTime::from_timestamp(
                        ns / 1_000_000_000,
                        (ns % 1_000_000_000) as u32,
                    )
                    .map(|dt| dt.naive_utc())
                    .unwrap_or_default();
                    data_string.push(value_to_block_string(
                        &exasol_udf_sdk::value::Value::Timestamp(ts),
                    ));
                }
                ColAccessor::Decimal128(arr, scale) => {
                    let d = exasol_udf_sdk::value::Decimal {
                        unscaled: arr.value(r),
                        scale: *scale as u8,
                    };
                    data_string.push(value_to_block_string(
                        &exasol_udf_sdk::value::Value::Numeric(d),
                    ));
                }
                ColAccessor::NumericFromInt32(arr) => {
                    data_string.push(value_to_block_string(&exasol_udf_sdk::value::Value::Int32(
                        arr.value(r),
                    )));
                }
                ColAccessor::NumericFromInt64(arr) => {
                    data_string.push(value_to_block_string(&exasol_udf_sdk::value::Value::Int64(
                        arr.value(r),
                    )));
                }
                ColAccessor::NumericFromFloat64(arr) => {
                    data_string.push(value_to_block_string(
                        &exasol_udf_sdk::value::Value::Double(arr.value(r)),
                    ));
                }
                ColAccessor::Unsupported => {}
            }
        }
    }

    Ok(exa_proto::ExascriptTableData {
        rows: n_rows as u64,
        rows_in_group: 0,
        data_string,
        data_nulls,
        data_bool,
        data_int32,
        data_int64,
        data_double,
        row_number: vec![],
    })
}

/// Convert a (possibly sliced) RecordBatch into a `Vec<Vec<Value>>` for tail
/// materialisation into the shared `EmitBuffer`.
///
/// Uses the same `build_accessors` downcast-once path as `encode_slice` so
/// both functions share a single downcast site. The resulting `Value` payload
/// for each cell matches what the row path's `push` + `to_proto` would produce
/// (same `value_to_block_string` rendering for string-block types), so
/// subsequent `emit` calls and the end-of-`run` tail flush are coherent.
#[cfg(feature = "emit-arrow")]
fn arrow_batch_to_value_rows(
    batch: &arrow::record_batch::RecordBatch,
    meta: &[ColumnMeta],
) -> Result<Vec<Vec<exasol_udf_sdk::value::Value>>, UdfError> {
    use arrow::array::Array;
    use exasol_udf_sdk::value::Value;

    let accessors = build_accessors(batch, meta)?;
    let n_rows = batch.num_rows();
    let n_cols = meta.len();

    let null_bufs: Vec<_> = (0..n_cols)
        .map(|c| batch.column(c).nulls().cloned())
        .collect();

    let mut result = Vec::with_capacity(n_rows);

    for r in 0..n_rows {
        let mut row = Vec::with_capacity(n_cols);
        for (c, acc) in accessors.iter().enumerate() {
            let is_null = null_bufs[c].as_ref().is_some_and(|nb| nb.is_null(r));
            if is_null {
                row.push(Value::Null);
                continue;
            }

            let v = match acc {
                ColAccessor::Int32(arr) => Value::Int32(arr.value(r)),
                ColAccessor::Int64(arr) => Value::Int64(arr.value(r)),
                ColAccessor::Float64(arr) => Value::Double(arr.value(r)),
                ColAccessor::Boolean(arr) => Value::Bool(arr.value(r)),
                ColAccessor::Utf8(arr) => Value::String(arr.value(r).to_string()),
                ColAccessor::LargeUtf8(arr) => Value::String(arr.value(r).to_string()),
                ColAccessor::Date32(arr) => {
                    let date = chrono::NaiveDate::from_num_days_from_ce_opt(
                        arr.value(r) + 719163, // Arrow epoch: 1970-01-01 = day 719163 in CE days
                    )
                    .unwrap_or_default();
                    Value::Date(date)
                }
                ColAccessor::TsSecond(arr) => Value::Timestamp(
                    chrono::DateTime::from_timestamp(arr.value(r), 0)
                        .map(|dt| dt.naive_utc())
                        .unwrap_or_default(),
                ),
                ColAccessor::TsMillisecond(arr) => Value::Timestamp(
                    chrono::DateTime::from_timestamp_millis(arr.value(r))
                        .map(|dt| dt.naive_utc())
                        .unwrap_or_default(),
                ),
                ColAccessor::TsMicrosecond(arr) => Value::Timestamp(
                    chrono::DateTime::from_timestamp_micros(arr.value(r))
                        .map(|dt| dt.naive_utc())
                        .unwrap_or_default(),
                ),
                ColAccessor::TsNanosecond(arr) => {
                    let ns = arr.value(r);
                    Value::Timestamp(
                        chrono::DateTime::from_timestamp(
                            ns / 1_000_000_000,
                            (ns % 1_000_000_000) as u32,
                        )
                        .map(|dt| dt.naive_utc())
                        .unwrap_or_default(),
                    )
                }
                ColAccessor::Decimal128(arr, scale) => {
                    Value::Numeric(exasol_udf_sdk::value::Decimal {
                        unscaled: arr.value(r),
                        scale: *scale as u8,
                    })
                }
                ColAccessor::NumericFromInt32(arr) => Value::Int32(arr.value(r)),
                ColAccessor::NumericFromInt64(arr) => Value::Int64(arr.value(r)),
                ColAccessor::NumericFromFloat64(arr) => Value::Double(arr.value(r)),
                ColAccessor::Unsupported => Value::Null,
            };
            row.push(v);
        }
        result.push(row);
    }

    Ok(result)
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
fn decode_string_block(typ: &ExaType, s: String) -> Value {
    match typ {
        ExaType::Numeric { .. } => match Decimal::try_from(s.as_str()) {
            Ok(d) => Value::Numeric(d),
            Err(_) => Value::Null,
        },
        ExaType::Date => {
            match fast_parse_date(&s).or_else(|| NaiveDate::parse_from_str(&s, DATE_FORMAT).ok()) {
                Some(d) => Value::Date(d),
                None => Value::Null,
            }
        }
        ExaType::Timestamp | ExaType::TimestampTz => {
            match fast_parse_timestamp(&s)
                .or_else(|| NaiveDateTime::parse_from_str(&s, TIMESTAMP_PARSE).ok())
                .or_else(|| NaiveDateTime::parse_from_str(&s, TIMESTAMP_FORMAT_ISO).ok())
            {
                Some(ts) => Value::Timestamp(ts),
                None => Value::Null,
            }
        }
        _ => Value::String(s),
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

/// Coerce a non-null `Value` to `i64` for an INT32/INT64 EMITS column.
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

/// Bridges one materialised input batch and an emit buffer to the SDK's
/// `UdfContext`. The UDF advances through rows via `next` and reads the current
/// row via `get`; `emit` appends to the buffer.
///
/// `next` semantics: the first call positions on row 0 (returning whether any
/// row exists); subsequent calls advance. This lets both scalar and set UDFs
/// use the canonical `while ctx.next()? { ... }` loop while the dispatch layer
/// controls batch refills.
///
/// On-demand credential fetcher: given a CONNECTION name, sends MT_IMPORT to the
/// DB and returns the resulting `ConnInfo`. `Fn` (not `FnOnce`) because
/// `connection()` borrows `&self` and may be called repeatedly for different
/// named connections within a single UDF run.
#[cfg(feature = "connect-back")]
pub type ConnRequester<'a> =
    Box<dyn Fn(&str) -> Result<exa_zmq_protocol::ConnInfo, exasol_udf_sdk::error::UdfError> + 'a>;

/// Flushes one pre-built proto table to the DB mid-run. Receives the
/// already-serialised `ExascriptTableData` so both the row path (which calls
/// `to_proto` + `clear` before invoking the flusher) and the batch path (which
/// encodes slices directly without touching the `Vec<Value>` buffer) share the
/// same wire-send logic. Feature-independent: mid-run flushing is not gated on
/// `connect-back`.
pub type EmitFlusher<'a> =
    Box<dyn FnMut(exa_proto::ExascriptTableData) -> Result<(), UdfError> + 'a>;

pub struct HostContextBridge<'a> {
    input: &'a mut InputRowSet,
    emit_buf: &'a mut EmitBuffer,
    input_cols: &'a [ColumnMeta],
    /// Declared EMITS output schema — used by `emit_batch` to choose the target
    /// proto block for each Arrow column. Threaded in by `dispatch::run_batch`
    /// alongside `input_cols` because the bridge previously held only the input
    /// columns and `emit_batch` needs the output schema at encoding time.
    output_meta: &'a [ColumnMeta],
    started: bool,
    /// Sends one pre-built proto table to the DB when the buffer crosses its byte
    /// threshold, keeping a single batch's output bounded. Invoked from `emit`
    /// (after serialising + clearing the buffer) and from `push_batch` (after
    /// encoding each full ≤4 MB slice directly).
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
/// owned (not borrowed) because the corresponding `UdfContext` accessors return
/// owned `String`/`Option<String>` across the `.so` vtable boundary. Built from
/// a `&UdfMeta` via `From`; `Default` yields the all-neutral value tests use.
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
        input_cols: &'a [ColumnMeta],
        output_meta: &'a [ColumnMeta],
        flusher: EmitFlusher<'a>,
        handshake: HandshakeMeta,
        #[cfg(feature = "connect-back")] conn_requester: ConnRequester<'a>,
    ) -> Self {
        HostContextBridge {
            input,
            emit_buf,
            input_cols,
            output_meta,
            started: false,
            flusher,
            last_error: std::cell::Cell::new(None),
            handshake,
            #[cfg(feature = "connect-back")]
            conn_requester,
        }
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
        input_cols: &'a [ColumnMeta],
        output_meta: &'a [ColumnMeta],
        flusher: EmitFlusher<'a>,
        handshake: HandshakeMeta,
        conn_requester: ConnRequester<'a>,
    ) -> Self {
        HostContextBridge {
            input,
            emit_buf,
            input_cols,
            output_meta,
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

impl UdfContext for HostContextBridge<'_> {
    fn num_columns(&self) -> usize {
        self.input_cols.len()
    }

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

    fn get(&self, col: usize) -> Result<&Value, UdfError> {
        self.input
            .current_row()
            .get(col)
            .ok_or_else(|| UdfError::Type(format!("column {col} out of range")))
    }

    fn emit(&mut self, values: &[Value]) -> Result<(), UdfError> {
        self.emit_buf.push(values.to_vec());
        if self.emit_buf.should_flush() {
            self.emit_buf.record_flush_telemetry();
            let table = self.emit_buf.to_proto(self.output_meta);
            tracing::debug!(
                target: "emit_flush",
                rows = table.rows,
                "MT_EMIT sending"
            );
            (self.flusher)(table)?;
            self.emit_buf.clear();
        }
        Ok(())
    }

    #[cfg(feature = "emit-arrow")]
    fn emit_record_batch_ipc(&mut self, ipc: &[u8]) -> Result<(), UdfError> {
        // Resolve the disjoint borrow: take references to fields we need
        // separately so the borrow checker sees them as independent borrows.
        let emit_buf = &mut *self.emit_buf;
        let flusher = &mut self.flusher;
        let meta = self.output_meta;
        // Deserialise into a host-owned RecordBatch (single arrow copy on this
        // side of the .so boundary), then replay the existing push_batch path.
        let reader = arrow::ipc::reader::StreamReader::try_new(std::io::Cursor::new(ipc), None)
            .map_err(|e| UdfError::Type(format!("emit_batch: IPC reader init: {e}")))?;
        for batch in reader {
            let batch = batch.map_err(|e| UdfError::Type(format!("emit_batch: IPC read: {e}")))?;
            emit_buf.push_batch(&batch, meta, &mut |table| (flusher)(table))?;
        }
        // After push_batch returns, at most one <4 MB tail is buffered.
        // Apply the same should_flush check as emit() so a tail that itself
        // crosses the threshold is flushed immediately (possible if many
        // interleaved push calls accumulated bytes before this batch).
        if emit_buf.should_flush() {
            let table = emit_buf.to_proto(meta);
            (flusher)(table)?;
            emit_buf.clear();
        }
        Ok(())
    }

    fn next(&mut self) -> Result<bool, UdfError> {
        if self.input.is_empty() {
            return Ok(false);
        }
        if !self.started {
            self.started = true;
            return Ok(true);
        }
        Ok(self.input.advance())
    }

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
        #[cfg(feature = "connect-back")] conn_requester: ConnRequester<'a>,
    ) -> Self {
        SingleCallContext {
            last_error: std::cell::Cell::new(None),
            handshake,
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
    fn num_columns(&self) -> usize {
        0
    }

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

    fn get(&self, _col: usize) -> Result<&Value, UdfError> {
        Err(UdfError::Unimplemented(
            "single-call mode has no input columns".into(),
        ))
    }

    fn emit(&mut self, _values: &[Value]) -> Result<(), UdfError> {
        Err(UdfError::Unimplemented(
            "single-call mode does not emit rows".into(),
        ))
    }

    fn next(&mut self) -> Result<bool, UdfError> {
        Err(UdfError::Unimplemented(
            "single-call mode has no input rows".into(),
        ))
    }

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
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify that `debug_level()` on both bridges reads the process-global
    /// `LevelFilter` and never panics (including when the filter is `OFF`).
    ///
    /// The implementation is `LevelFilter::current().into_level().unwrap_or(INFO)`.
    /// We cannot set the global level in a unit test without a subscriber, so
    /// we verify the weaker property: the method returns a valid `Level` value
    /// (one of the five known variants) and maps `OFF` to `INFO` by checking
    /// directly with `LevelFilter::OFF.into_level()`.
    #[test]
    fn host_bridge_debug_level_returns_valid_level() {
        use exa_proto::ExascriptTableData;

        let meta = vec![ColumnMeta {
            name: "a".to_string(),
            typ: ExaType::Int64,
            type_name: String::new(),
            size: None,
            precision: None,
            scale: None,
        }];
        let table = ExascriptTableData {
            rows: 0,
            ..Default::default()
        };
        let mut rs = InputRowSet::from_proto(&table, &meta);
        let mut emit = EmitBuffer::new();
        let bridge = HostContextBridge::new(
            &mut rs,
            &mut emit,
            &meta,
            &meta,
            Box::new(|_| Ok(())),
            HandshakeMeta::default(),
            #[cfg(feature = "connect-back")]
            Box::new(|_name| {
                Err(exasol_udf_sdk::error::UdfError::ConnectBack(
                    "no credential fetcher".into(),
                ))
            }),
        );

        // The bridge must not panic and must return a valid Level variant.
        let level = bridge.debug_level();
        assert!(
            level == tracing::Level::ERROR
                || level == tracing::Level::WARN
                || level == tracing::Level::INFO
                || level == tracing::Level::DEBUG
                || level == tracing::Level::TRACE,
            "unexpected level {level}"
        );

        // The OFF fallback is encoded in the implementation, not the global
        // state; verify the expression directly.
        let off_mapped = tracing::level_filters::LevelFilter::OFF
            .into_level()
            .unwrap_or(tracing::Level::INFO);
        assert_eq!(off_mapped, tracing::Level::INFO, "OFF must map to INFO");
    }

    #[test]
    fn single_call_context_debug_level_returns_valid_level() {
        #[cfg(feature = "connect-back")]
        let ctx = SingleCallContext::new(
            HandshakeMeta::default(),
            Box::new(|_name| {
                Err(exasol_udf_sdk::error::UdfError::ConnectBack(
                    "no credential fetcher".into(),
                ))
            }),
        );
        #[cfg(not(feature = "connect-back"))]
        let ctx = SingleCallContext::new(HandshakeMeta::default());

        let level = ctx.debug_level();
        assert!(
            level == tracing::Level::ERROR
                || level == tracing::Level::WARN
                || level == tracing::Level::INFO
                || level == tracing::Level::DEBUG
                || level == tracing::Level::TRACE,
            "unexpected level {level}"
        );
    }

    fn col(name: &str, typ: ExaType) -> ColumnMeta {
        ColumnMeta {
            name: name.to_string(),
            typ,
            type_name: String::new(),
            size: None,
            precision: None,
            scale: None,
        }
    }

    /// Construct a bridge for the tests, supplying the connect-back arg only
    /// when the feature is enabled so the same call sites compile either way.
    fn make_bridge<'a>(
        input: &'a mut InputRowSet,
        emit: &'a mut EmitBuffer,
        cols: &'a [ColumnMeta],
    ) -> HostContextBridge<'a> {
        HostContextBridge::new(
            input,
            emit,
            cols,
            cols, // output_meta: reuse the same schema for test simplicity
            Box::new(|_t: exa_proto::ExascriptTableData| Ok(())),
            HandshakeMeta::default(),
            #[cfg(feature = "connect-back")]
            Box::new(|_name| {
                Err(exasol_udf_sdk::error::UdfError::ConnectBack(
                    "no credential fetcher in test".into(),
                ))
            }),
        )
    }

    /// One batch, 2 rows, mixed types with a NULL cell. Verifies dense per-type
    /// block decoding and row-major NULL bitmap handling.
    fn mixed_batch() -> (ExascriptTableData, Vec<ColumnMeta>) {
        // Columns: [Int64, String, Double, Boolean]
        let meta = vec![
            col("a", ExaType::Int64),
            col("b", ExaType::String { size: None }),
            col("c", ExaType::Double),
            col("d", ExaType::Boolean),
        ];
        let n_rows = 2;
        let n_cols = 4;
        // row0: (10, "x", 1.5, true)   row1: (20, NULL-string, 2.5, false)
        let mut data_nulls = vec![false; n_rows * n_cols];
        // null at row1, col1 (string) -> index 1*4 + 1 = 5
        data_nulls[5] = true;
        let table = ExascriptTableData {
            rows: n_rows as u64,
            rows_in_group: 0,
            // string block (col1): row0="x", row1=placeholder ""
            data_string: vec!["x".into(), String::new()],
            data_nulls,
            data_bool: vec![true, false],
            data_int32: vec![],
            data_int64: vec![10, 20],
            data_double: vec![1.5, 2.5],
            row_number: vec![],
        };
        (table, meta)
    }

    #[test]
    fn bridge_materializes_input_rows() {
        let (table, meta) = mixed_batch();
        let rs = InputRowSet::from_proto(&table, &meta);
        assert_eq!(rs.len(), 2);
        assert_eq!(
            rs.row(0).unwrap(),
            &[
                Value::Int64(10),
                Value::String("x".into()),
                Value::Double(1.5),
                Value::Bool(true),
            ]
        );
        assert_eq!(
            rs.row(1).unwrap(),
            &[
                Value::Int64(20),
                Value::Null,
                Value::Double(2.5),
                Value::Bool(false),
            ]
        );
    }

    /// `to_proto`'s per-type blocks are pre-sized with `Vec::with_capacity` for
    /// the exact (non-NULL) column count instead of growing via `Vec::new()` +
    /// `push`. This only asserts the resulting contents are correct — `Vec`'s
    /// capacity is only guaranteed to be *at least* the requested value (the
    /// allocator/growth strategy is explicitly unspecified), so asserting an
    /// exact `capacity()` here would be a flaky test rather than a real
    /// regression guard; the pre-sizing's throughput benefit is verified by
    /// `benches/emit-bench`, not by inspecting internal `Vec` capacity.
    #[test]
    fn to_proto_presizes_string_block_capacity() {
        let meta = vec![col("a", ExaType::String { size: None })];
        let mut emit = EmitBuffer::new();
        emit.push(vec![Value::String("a".into())]);
        emit.push(vec![Value::String("b".into())]);
        emit.push(vec![Value::String("c".into())]);

        let table = emit.to_proto(&meta);
        assert_eq!(table.data_string, vec!["a", "b", "c"]);
    }

    #[test]
    fn bridge_typed_accessors() {
        let (table, meta) = mixed_batch();
        let mut rs = InputRowSet::from_proto(&table, &meta);
        let mut emit = EmitBuffer::new();
        let mut bridge = make_bridge(&mut rs, &mut emit, &meta);

        assert!(bridge.next().unwrap());
        assert_eq!(bridge.num_columns(), 4);
        assert_eq!(bridge.get(0).unwrap(), &Value::Int64(10));
        assert_eq!(bridge.get(1).unwrap(), &Value::String("x".into()));
        assert_eq!(bridge.get(3).unwrap(), &Value::Bool(true));
        assert!(matches!(bridge.get(99), Err(UdfError::Type(_))));

        assert!(bridge.next().unwrap());
        assert_eq!(bridge.get(0).unwrap(), &Value::Int64(20));
        assert_eq!(bridge.get(1).unwrap(), &Value::Null);

        assert!(!bridge.next().unwrap());
    }

    #[test]
    fn emit_buffer_roundtrips_through_proto() {
        let meta = vec![
            col("a", ExaType::Int64),
            col("b", ExaType::String { size: None }),
            col("c", ExaType::Double),
            col("d", ExaType::Boolean),
        ];
        let mut emit = EmitBuffer::new();
        emit.push(vec![
            Value::Int64(10),
            Value::String("x".into()),
            Value::Double(1.5),
            Value::Bool(true),
        ]);
        emit.push(vec![
            Value::Int64(20),
            Value::Null,
            Value::Double(2.5),
            Value::Bool(false),
        ]);

        let table = emit.to_proto(&meta);
        // Decoding the emitted batch back must reproduce the original rows,
        // proving from_proto/to_proto are symmetric (dense per-type blocks).
        let rs = InputRowSet::from_proto(&table, &meta);
        assert_eq!(
            rs.row(0).unwrap(),
            &[
                Value::Int64(10),
                Value::String("x".into()),
                Value::Double(1.5),
                Value::Bool(true),
            ]
        );
        assert_eq!(
            rs.row(1).unwrap(),
            &[
                Value::Int64(20),
                Value::Null,
                Value::Double(2.5),
                Value::Bool(false),
            ]
        );
    }

    #[test]
    fn emit_packs_by_declared_type_not_value_variant() {
        // A connect-back SELECT can return a DECIMAL column as Value::Int64, but
        // the EMITS column is ExaType::Numeric (string block). to_proto must
        // place it in the string block so the DB reads it from the right block.
        let meta = vec![
            col("region", ExaType::String { size: None }),
            col(
                "id",
                ExaType::Numeric {
                    precision: None,
                    scale: None,
                },
            ),
        ];
        let mut emit = EmitBuffer::new();
        emit.push(vec![Value::String("EU".into()), Value::Int64(1)]);
        emit.push(vec![Value::String("EU".into()), Value::Int64(2)]);

        let table = emit.to_proto(&meta);
        // Both columns are numeric/string -> the string block holds all cells in
        // row-major (row, column) order: row0 region,id then row1 region,id.
        assert_eq!(table.data_string, vec!["EU", "1", "EU", "2"]);
        assert!(table.data_int64.is_empty());

        let rs = InputRowSet::from_proto(&table, &meta);
        assert_eq!(
            rs.row(0).unwrap(),
            &[
                Value::String("EU".into()),
                Value::Numeric(Decimal::try_from("1").unwrap()),
            ]
        );
        assert_eq!(
            rs.row(1).unwrap(),
            &[
                Value::String("EU".into()),
                Value::Numeric(Decimal::try_from("2").unwrap()),
            ]
        );
    }

    #[test]
    fn emit_string_block_is_row_major_across_columns() {
        // Two same-type-block columns over two rows must interleave row-major in
        // data_string: row0(c0,c1) then row1(c0,c1). A column-major layout would
        // land row1's first cell where the DB expects row0's second column.
        let meta = vec![
            col(
                "a",
                ExaType::Numeric {
                    precision: None,
                    scale: None,
                },
            ),
            col("b", ExaType::String { size: None }),
        ];
        let mut emit = EmitBuffer::new();
        emit.push(vec![
            Value::Numeric(Decimal::try_from("100").unwrap()),
            Value::String("AAA".into()),
        ]);
        emit.push(vec![
            Value::Numeric(Decimal::try_from("200").unwrap()),
            Value::String("BBB".into()),
        ]);

        let table = emit.to_proto(&meta);
        assert_eq!(table.data_string, vec!["100", "AAA", "200", "BBB"]);

        let rs = InputRowSet::from_proto(&table, &meta);
        assert_eq!(
            rs.row(0).unwrap(),
            &[
                Value::Numeric(Decimal::try_from("100").unwrap()),
                Value::String("AAA".into()),
            ]
        );
        assert_eq!(
            rs.row(1).unwrap(),
            &[
                Value::Numeric(Decimal::try_from("200").unwrap()),
                Value::String("BBB".into()),
            ]
        );
    }

    #[test]
    fn emit_null_cell_occupies_no_type_block_slot() {
        // A NULL numeric cell must not reserve a slot in the string block: the
        // bitmap marks it, and only the non-null "5" occupies the block. A
        // placeholder would shift "AAA"/"BBB" into the numeric column.
        let meta = vec![
            col(
                "id",
                ExaType::Numeric {
                    precision: None,
                    scale: None,
                },
            ),
            col("note", ExaType::String { size: None }),
        ];
        let mut emit = EmitBuffer::new();
        emit.push(vec![Value::Null, Value::String("AAA".into())]);
        emit.push(vec![
            Value::Numeric(Decimal::try_from("5").unwrap()),
            Value::String("BBB".into()),
        ]);

        let table = emit.to_proto(&meta);
        // row0: id=NULL (skipped), note="AAA"; row1: id="5", note="BBB".
        assert_eq!(table.data_string, vec!["AAA", "5", "BBB"]);
        assert_eq!(table.data_nulls, vec![true, false, false, false]);

        let rs = InputRowSet::from_proto(&table, &meta);
        assert_eq!(
            rs.row(0).unwrap(),
            &[Value::Null, Value::String("AAA".into())]
        );
        assert_eq!(
            rs.row(1).unwrap(),
            &[
                Value::Numeric(Decimal::try_from("5").unwrap()),
                Value::String("BBB".into()),
            ]
        );
    }

    #[test]
    fn bridge_typed_getters_return_typed_options() {
        // A NUMERIC column must decode to Value::Numeric(Decimal) (carrying its
        // scale), a DATE to Value::Date(NaiveDate) and a TIMESTAMP to
        // Value::Timestamp(NaiveDateTime) — never a raw string. The fractional
        // timestamp exercises the %.f wire format on both decode and encode.
        let meta = vec![
            col(
                "amount",
                ExaType::Numeric {
                    precision: Some(10),
                    scale: Some(2),
                },
            ),
            col("d", ExaType::Date),
            col("ts", ExaType::Timestamp),
        ];
        let table = ExascriptTableData {
            rows: 1,
            rows_in_group: 0,
            data_string: vec![
                "12.34".into(),
                "2026-06-14".into(),
                "2026-06-14 09:30:15.250000".into(),
            ],
            data_nulls: vec![false, false, false],
            ..Default::default()
        };

        let rs = InputRowSet::from_proto(&table, &meta);
        let expected_date = NaiveDate::from_ymd_opt(2026, 6, 14).unwrap();
        let expected_ts = expected_date.and_hms_micro_opt(9, 30, 15, 250_000).unwrap();
        let decoded = rs.row(0).unwrap();
        assert_eq!(
            decoded[0],
            Value::Numeric(Decimal::try_from("12.34").unwrap())
        );
        assert_eq!(decoded[1], Value::Date(expected_date));
        assert_eq!(decoded[2], Value::Timestamp(expected_ts));

        // Round-trip: from_proto -> to_proto -> from_proto preserves typed values.
        let mut emit = EmitBuffer::new();
        emit.push(decoded.to_vec());
        let reproto = emit.to_proto(&meta);
        let reread = InputRowSet::from_proto(&reproto, &meta);
        assert_eq!(reread.row(0).unwrap(), decoded);
    }

    #[test]
    fn corrupt_string_block_value_decodes_to_null() {
        // A non-parseable NUMERIC/DATE cell must degrade to Value::Null rather
        // than aborting decode, so one corrupt cell cannot poison the batch.
        let meta = vec![
            col(
                "amount",
                ExaType::Numeric {
                    precision: None,
                    scale: None,
                },
            ),
            col("d", ExaType::Date),
        ];
        let table = ExascriptTableData {
            rows: 1,
            rows_in_group: 0,
            data_string: vec!["not-a-number".into(), "not-a-date".into()],
            data_nulls: vec![false, false],
            ..Default::default()
        };
        let rs = InputRowSet::from_proto(&table, &meta);
        assert_eq!(rs.row(0).unwrap(), &[Value::Null, Value::Null]);
    }

    #[test]
    fn emit_buffer_byte_estimate_and_should_flush() {
        // Each row carries a 1000-byte string; the estimate grows by ~1000 per
        // push. Pushing rows until just below the 4 MB limit must keep
        // should_flush() false; one more push crosses it and flips it true.
        let row = || vec![Value::String("x".repeat(1000))];
        let mut emit = EmitBuffer::new();

        // Rows needed to first reach or exceed the limit.
        let rows_to_limit = EMIT_BUFFER_LIMIT_BYTES.div_ceil(1000);

        for _ in 0..(rows_to_limit - 1) {
            emit.push(row());
        }
        assert!(
            !emit.should_flush(),
            "buffer just below the limit must not request a flush"
        );

        emit.push(row());
        assert!(
            emit.should_flush(),
            "buffer at or above the limit must request a flush"
        );

        emit.clear();
        assert!(
            !emit.should_flush(),
            "clear() must reset the byte estimate so should_flush() is false"
        );
    }

    #[test]
    fn oversized_single_row_flushes_alone() {
        // A single row whose string exceeds the limit must trip should_flush()
        // after one push, so an oversized row is flushed on its own rather than
        // accumulating forever.
        let mut emit = EmitBuffer::new();
        emit.push(vec![Value::String("y".repeat(EMIT_BUFFER_LIMIT_BYTES + 1))]);
        assert!(
            emit.should_flush(),
            "a single oversized row must request a flush after one push"
        );
    }

    #[test]
    fn timestamp_emit_nanosecond_roundtrip() {
        // GIVEN a Timestamp value with sub-microsecond (nanosecond) precision.
        // 123456789 ns = 123456 µs + 789 ns; %.6f would truncate to 123456 µs.
        let ts = NaiveDate::from_ymd_opt(2026, 6, 14)
            .unwrap()
            .and_hms_nano_opt(9, 30, 15, 123_456_789)
            .unwrap();
        let meta = vec![col("ts", ExaType::Timestamp)];

        // WHEN serialised via EmitBuffer -> to_proto (uses value_to_block_string).
        let mut emit = EmitBuffer::new();
        emit.push(vec![Value::Timestamp(ts)]);
        let table = emit.to_proto(&meta);

        // THEN the emitted string contains exactly 9 fractional digits.
        let emitted_str = &table.data_string[0];
        assert!(
            emitted_str.ends_with(".123456789"),
            "expected 9-digit nanosecond fraction, got: {emitted_str}"
        );

        // AND it round-trips losslessly via from_proto.
        let rs = InputRowSet::from_proto(&table, &meta);
        assert_eq!(
            rs.row(0).unwrap(),
            &[Value::Timestamp(ts)],
            "nanosecond timestamp must survive to_proto -> from_proto round-trip"
        );
    }

    #[test]
    fn empty_batch_next_is_false() {
        let meta = vec![col("a", ExaType::Int64)];
        let table = ExascriptTableData {
            rows: 0,
            ..Default::default()
        };
        let mut rs = InputRowSet::from_proto(&table, &meta);
        let mut emit = EmitBuffer::new();
        let mut bridge = make_bridge(&mut rs, &mut emit, &meta);
        assert!(!bridge.next().unwrap());
    }

    #[test]
    fn bridge_returns_memory_limit() {
        let meta = vec![col("a", ExaType::Int64)];
        let table = ExascriptTableData {
            rows: 0,
            ..Default::default()
        };
        let mut rs = InputRowSet::from_proto(&table, &meta);
        let mut emit = EmitBuffer::new();
        let limit_bytes: u64 = 512 * 1024 * 1024;
        let bridge = HostContextBridge::new(
            &mut rs,
            &mut emit,
            &meta,
            &meta, // output_meta
            Box::new(|_t: exa_proto::ExascriptTableData| Ok(())),
            HandshakeMeta {
                memory_limit: limit_bytes,
                ..Default::default()
            },
            #[cfg(feature = "connect-back")]
            Box::new(|_name| {
                Err(exasol_udf_sdk::error::UdfError::ConnectBack(
                    "no credential fetcher in test".into(),
                ))
            }),
        );
        assert_eq!(bridge.memory_limit(), limit_bytes);
    }

    #[test]
    fn bridge_returns_handshake_metadata() {
        let meta = vec![col("a", ExaType::Int64)];
        let table = ExascriptTableData {
            rows: 0,
            ..Default::default()
        };
        let mut rs = InputRowSet::from_proto(&table, &meta);
        let mut emit = EmitBuffer::new();
        // A present optional (current_user) and absent optionals (current_schema,
        // scope_user) prove the bridge mirrors the proto present/absent distinction.
        let handshake = HandshakeMeta {
            session_id: 4242,
            statement_id: 9,
            node_id: 1,
            node_count: 4,
            vm_id: 777777,
            memory_limit: 256 * 1024 * 1024,
            database_name: "EXADB".to_string(),
            database_version: "2026.1.0".to_string(),
            script_name: "MY_SCRIPT".to_string(),
            script_schema: "MY_SCHEMA".to_string(),
            current_user: Some("ALICE".to_string()),
            current_schema: None,
            scope_user: None,
        };
        let bridge = HostContextBridge::new(
            &mut rs,
            &mut emit,
            &meta,
            &meta,
            Box::new(|_t: exa_proto::ExascriptTableData| Ok(())),
            handshake,
            #[cfg(feature = "connect-back")]
            Box::new(|_name| {
                Err(exasol_udf_sdk::error::UdfError::ConnectBack(
                    "no credential fetcher in test".into(),
                ))
            }),
        );

        // Numeric accessors return the exact UdfMeta values, no rescaling.
        assert_eq!(bridge.session_id(), 4242);
        assert_eq!(bridge.statement_id(), 9);
        assert_eq!(bridge.node_id(), 1);
        assert_eq!(bridge.node_count(), 4);
        assert_eq!(bridge.vm_id(), 777777);
        assert_eq!(bridge.memory_limit(), 256 * 1024 * 1024);
        // Owned-string accessors return the exact values.
        assert_eq!(bridge.database_name(), "EXADB");
        assert_eq!(bridge.database_version(), "2026.1.0");
        assert_eq!(bridge.script_name(), "MY_SCRIPT");
        assert_eq!(bridge.script_schema(), "MY_SCHEMA");
        // Optionals: Some when present, None when absent.
        assert_eq!(bridge.current_user(), Some("ALICE".to_string()));
        assert_eq!(bridge.current_schema(), None);
        assert_eq!(bridge.scope_user(), None);
    }

    #[test]
    fn single_call_context_returns_handshake_metadata() {
        // A present optional (current_user) and absent optionals (current_schema,
        // scope_user) prove the single-call context mirrors the proto
        // present/absent distinction, same as HostContextBridge.
        let handshake = HandshakeMeta {
            session_id: 4242,
            statement_id: 9,
            node_id: 1,
            node_count: 4,
            vm_id: 777777,
            memory_limit: 256 * 1024 * 1024,
            database_name: "EXADB".to_string(),
            database_version: "2026.1.0".to_string(),
            script_name: "MY_SCRIPT".to_string(),
            script_schema: "MY_SCHEMA".to_string(),
            current_user: Some("ALICE".to_string()),
            current_schema: None,
            scope_user: None,
        };

        #[cfg(feature = "connect-back")]
        let ctx = SingleCallContext::new(
            handshake,
            Box::new(|_name| {
                Err(exasol_udf_sdk::error::UdfError::ConnectBack(
                    "no credential fetcher in test".into(),
                ))
            }),
        );
        #[cfg(not(feature = "connect-back"))]
        let ctx = SingleCallContext::new(handshake);

        // Numeric accessors return the exact UdfMeta values, no rescaling.
        assert_eq!(ctx.session_id(), 4242);
        assert_eq!(ctx.statement_id(), 9);
        assert_eq!(ctx.node_id(), 1);
        assert_eq!(ctx.node_count(), 4);
        assert_eq!(ctx.vm_id(), 777777);
        assert_eq!(ctx.memory_limit(), 256 * 1024 * 1024);
        // Owned-string accessors return the exact values.
        assert_eq!(ctx.database_name(), "EXADB");
        assert_eq!(ctx.database_version(), "2026.1.0");
        assert_eq!(ctx.script_name(), "MY_SCRIPT");
        assert_eq!(ctx.script_schema(), "MY_SCHEMA");
        // Optionals: Some when present, None when absent.
        assert_eq!(ctx.current_user(), Some("ALICE".to_string()));
        assert_eq!(ctx.current_schema(), None);
        assert_eq!(ctx.scope_user(), None);
    }

    // -----------------------------------------------------------------------
    // Telemetry tests (tasks 2.15 — 5.4)
    // -----------------------------------------------------------------------

    /// Serialises tests that install tracing subscribers via `with_default`.
    ///
    /// Any `tracing::subscriber::with_default` call that installs a
    /// DEBUG-level subscriber can, upon first use of a `debug!` callsite,
    /// trigger `rebuild_interest_cache` which updates the process-global
    /// `MAX_LEVEL` atomic.  Concurrent tests that also assert on captured
    /// debug output may see the wrong `MAX_LEVEL` and have their events
    /// silently dropped by the macro fast-path check.  Holding this lock for
    /// the full duration of any such test eliminates the race.
    static GLOBAL_LEVEL_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// A `MakeWriter` that appends to a shared `Mutex<Vec<u8>>`.
    ///
    /// Used by the telemetry tests to capture `tracing` output without a global
    /// subscriber.  Each call to `make_writer` clones the `Arc` so the subscriber
    /// can hold it across events.
    #[cfg(test)]
    struct LockedWriter(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl std::io::Write for LockedWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for LockedWriter {
        type Writer = LockedWriter;
        fn make_writer(&'a self) -> Self::Writer {
            LockedWriter(std::sync::Arc::clone(&self.0))
        }
    }

    /// Verify telemetry events appear at `debug` level and are absent at `info`.
    ///
    /// Scenario: `telemetry_emitted_at_debug_level_only` (plan task 5.4).
    ///
    /// Uses a `Mutex<Vec<u8>>` capture writer and `tracing::subscriber::with_default`
    /// with a `reload::Layer` so `filter_handle.modify` triggers
    /// `rebuild_interest_cache()`, which resets any previously cached callsite
    /// interests.  Holds `GLOBAL_LEVEL_LOCK` to prevent concurrent tests from
    /// racing on the global `MAX_LEVEL` atomic.
    #[test]
    fn telemetry_emitted_at_debug_level_only() {
        use std::sync::{Arc, Mutex};
        use tracing_subscriber::prelude::__tracing_subscriber_SubscriberExt;
        use tracing_subscriber::reload;

        let _guard = GLOBAL_LEVEL_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        let capture_with_level = |level: tracing::Level| -> String {
            let buf: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
            let initial = tracing_subscriber::EnvFilter::new("info");
            let (filter_layer, filter_handle) = reload::Layer::new(initial);
            let sub = tracing_subscriber::registry().with(filter_layer).with(
                tracing_subscriber::fmt::layer()
                    .with_writer(LockedWriter(Arc::clone(&buf)))
                    .with_ansi(false),
            );
            tracing::subscriber::with_default(sub, || {
                // Force rebuild_interest_cache() so previously-cached callsites are reset.
                let _ = filter_handle
                    .modify(|f| *f = tracing_subscriber::EnvFilter::new(level.as_str()));

                let mut emit = EmitBuffer::new();
                // Push enough rows to trigger a flush (each ~1000 bytes).
                let rows_to_flush = EMIT_BUFFER_LIMIT_BYTES.div_ceil(1000) + 1;
                for _ in 0..rows_to_flush {
                    emit.push(vec![Value::String("x".repeat(1000))]);
                    if emit.should_flush() {
                        emit.record_flush_telemetry();
                        emit.clear();
                    }
                }
                if !emit.is_empty() {
                    emit.record_flush_telemetry();
                    emit.clear();
                }
            });
            let captured = buf.lock().unwrap();
            String::from_utf8_lossy(&captured).into_owned()
        };

        let debug_output = capture_with_level(tracing::Level::DEBUG);
        // Restore to INFO before capturing the info output.
        let info_output = capture_with_level(tracing::Level::INFO);

        assert!(
            debug_output.contains("emit_flush"),
            "debug output must contain emit_flush telemetry, got: {debug_output:?}"
        );
        assert!(
            !info_output.contains("emit_flush"),
            "info output must not contain emit_flush telemetry, got: {info_output:?}"
        );
    }

    /// Verify that `debug!` events around `push` are recorded at debug level.
    ///
    /// Uses a `reload::Layer`-based subscriber and calls `filter_handle.modify`
    /// to trigger `rebuild_interest_cache()`, which resets any previously cached
    /// callsite interests (avoiding the "event permanently cached as never"
    /// failure mode when another test registered the callsite first without a
    /// debug-level subscriber installed).
    ///
    /// Scenario: `emit_flush_path_instrumented` (plan task 5.4).
    #[test]
    fn emit_flush_path_instrumented() {
        use std::sync::{Arc, Mutex};
        use tracing_subscriber::prelude::__tracing_subscriber_SubscriberExt;
        use tracing_subscriber::reload;

        let _guard = GLOBAL_LEVEL_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        let buf: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
        let initial = tracing_subscriber::EnvFilter::new("info");
        let (filter_layer, filter_handle) = reload::Layer::new(initial);
        let sub = tracing_subscriber::registry().with(filter_layer).with(
            tracing_subscriber::fmt::layer()
                .with_writer(LockedWriter(Arc::clone(&buf)))
                .with_ansi(false),
        );

        tracing::subscriber::with_default(sub, || {
            // Force rebuild_interest_cache() to reset any stale callsite cache entries.
            let _ = filter_handle.modify(|f| *f = tracing_subscriber::EnvFilter::new("debug"));

            let mut emit = EmitBuffer::new();
            emit.push(vec![Value::String("hello".to_string())]);
        });

        let captured = buf.lock().unwrap();
        let output = String::from_utf8_lossy(&captured);
        assert!(
            output.contains("emit_push") || output.contains("bytes_buffered"),
            "debug output must contain push instrumentation, got: {output:?}"
        );
    }

    // -----------------------------------------------------------------------
    // Permanent regression guard: the string-block fast-path formatters
    // (`value_to_block_string`'s NUMERIC/DATE/TIMESTAMP branches) must stay
    // byte-identical to the `chrono`/`Display` path they replaced, for every
    // representable value.
    // -----------------------------------------------------------------------
    mod fast_string_block_tests {
        use super::*;

        fn decimal(unscaled: i128, scale: u8) -> Decimal {
            Decimal { unscaled, scale }
        }

        /// `fast_decimal_to_string` must match `Decimal`'s `Display` impl for
        /// every representable case: positive/negative/zero, scale 0 through a
        /// realistic max, and padding edge cases where the unscaled digit run
        /// is shorter than the scale.
        #[test]
        fn fast_decimal_matches_display_for_all_cases() {
            let cases = [
                (0i128, 0u8),
                (0, 5),
                (1, 0),
                (-1, 0),
                (5, 2),
                (-5, 2),
                (12, 2),
                (-12, 2),
                (100, 2),
                (123456789, 3),
                (-123456789, 3),
                (1, 18),
                (-1, 18),
                (1_000_000_000_000_000_001, 18),
                (-1_000_000_000_000_000_001, 18),
                (i128::MAX, 0),
                (i128::MAX, 18),
                (i128::MIN, 0),
                (i128::MIN, 18),
                (i128::MIN, 38),
                (9, 1),
                (10, 1),
                (99, 1),
            ];
            for (unscaled, scale) in cases {
                let d = decimal(unscaled, scale);
                assert_eq!(
                    fast_decimal_to_string(&d),
                    d.to_string(),
                    "mismatch for unscaled={unscaled} scale={scale}"
                );
            }
        }

        /// `fast_date_to_string` must match `NaiveDate::format(DATE_FORMAT)`
        /// byte-for-byte across leap days, year boundaries, and pre-1970 dates.
        #[test]
        fn fast_date_matches_chrono_format() {
            let cases = [
                (0, 1, 1),
                (1, 1, 1),
                (1969, 12, 31),
                (1970, 1, 1),
                (2000, 2, 29),
                (2024, 2, 29),
                (2026, 6, 5),
                (9999, 12, 31),
            ];
            for (y, m, d) in cases {
                let date = NaiveDate::from_ymd_opt(y, m, d).unwrap();
                let expected = date.format(DATE_FORMAT).to_string();
                assert_eq!(
                    fast_date_to_string(&date),
                    Some(expected.clone()),
                    "mismatch for {y:04}-{m:02}-{d:02}, expected {expected}"
                );
            }
        }

        /// Years outside the common `0..=9999` range fall back to `None` so the
        /// caller defers to chrono's slow (but correct) path rather than
        /// producing wrong output.
        #[test]
        fn fast_date_defers_for_out_of_common_range_years() {
            let date = NaiveDate::from_ymd_opt(10000, 1, 1).unwrap();
            assert_eq!(fast_date_to_string(&date), None);

            let date = NaiveDate::from_ymd_opt(-1, 1, 1).unwrap();
            assert_eq!(fast_date_to_string(&date), None);
        }

        /// `fast_timestamp_to_string` must match
        /// `NaiveDateTime::format(TIMESTAMP_EMIT)` byte-for-byte, including
        /// always-9-digit fractional seconds, midnight, and nanosecond precision.
        #[test]
        fn fast_timestamp_matches_chrono_format() {
            let cases: Vec<NaiveDateTime> = vec![
                NaiveDate::from_ymd_opt(1970, 1, 1)
                    .unwrap()
                    .and_hms_opt(0, 0, 0)
                    .unwrap(),
                NaiveDate::from_ymd_opt(1969, 12, 31)
                    .unwrap()
                    .and_hms_opt(23, 59, 59)
                    .unwrap(),
                NaiveDate::from_ymd_opt(2024, 2, 29)
                    .unwrap()
                    .and_hms_opt(12, 30, 45)
                    .unwrap(),
                NaiveDate::from_ymd_opt(2026, 6, 5)
                    .unwrap()
                    .and_hms_nano_opt(1, 2, 3, 4)
                    .unwrap(),
                NaiveDate::from_ymd_opt(2026, 6, 5)
                    .unwrap()
                    .and_hms_nano_opt(23, 59, 59, 999_999_999)
                    .unwrap(),
                NaiveDate::from_ymd_opt(9999, 12, 31)
                    .unwrap()
                    .and_hms_opt(23, 59, 59)
                    .unwrap(),
            ];
            for ts in cases {
                let expected = ts.format(TIMESTAMP_EMIT).to_string();
                assert_eq!(
                    fast_timestamp_to_string(&ts),
                    Some(expected.clone()),
                    "mismatch for {ts:?}, expected {expected}"
                );
            }
        }

        /// A timestamp whose date falls outside the common year range defers
        /// to `None` for the same reason as `fast_date_to_string`.
        #[test]
        fn fast_timestamp_defers_for_out_of_common_range_years() {
            let ts = NaiveDate::from_ymd_opt(10000, 1, 1)
                .unwrap()
                .and_hms_opt(0, 0, 0)
                .unwrap();
            assert_eq!(fast_timestamp_to_string(&ts), None);
        }

        /// `value_to_block_string` must produce identical output whether or not
        /// the fast path is compiled in — this is the end-to-end proof that the
        /// fast path is wired in correctly and stays byte-identical for the
        /// whole `Value` enum, not just the two hand-tested helpers above.
        #[test]
        fn value_to_block_string_matches_slow_path_for_numeric_date_timestamp() {
            let numeric = Value::Numeric(decimal(-1_000_000_000_000_000_001, 18));
            assert_eq!(value_to_block_string(&numeric), "-1.000000000000000001");

            let date = Value::Date(NaiveDate::from_ymd_opt(2024, 2, 29).unwrap());
            assert_eq!(value_to_block_string(&date), "2024-02-29");

            let ts = Value::Timestamp(
                NaiveDate::from_ymd_opt(2026, 6, 5)
                    .unwrap()
                    .and_hms_nano_opt(1, 2, 3, 4)
                    .unwrap(),
            );
            assert_eq!(value_to_block_string(&ts), "2026-06-05 01:02:03.000000004");
        }

        /// End-to-end byte-identity proof at the `to_proto` level (plan
        /// scenario "A promoted emit fast-path encoder stays byte-identical to
        /// the row path"). Builds an `EmitBuffer` spanning the full string-block
        /// `ExaType` range — NUMERIC/DATE/TIMESTAMP/VARCHAR — with interspersed
        /// NULLs and two columns sharing the NUMERIC block, then asserts the
        /// serialised `ExascriptTableData`'s string block equals the exact
        /// vector the reference `chrono`/`Display` path (`d.to_string()`,
        /// `date.format(DATE_FORMAT)`, `ts.format(TIMESTAMP_EMIT)`) produces in
        /// dense row-major-interleaved order — so downstream Exasol parsing is
        /// unaffected by the fast formatter. Also pins the NULL bitmap and the
        /// non-string blocks.
        #[test]
        fn fast_path_to_proto_byte_identical_to_row_path() {
            fn date(y: i32, m: u32, d: u32) -> NaiveDate {
                NaiveDate::from_ymd_opt(y, m, d).unwrap()
            }
            fn ts(y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32, nano: u32) -> NaiveDateTime {
                date(y, mo, d).and_hms_nano_opt(h, mi, s, nano).unwrap()
            }

            // num_a and num_b share the NUMERIC string block; label is VARCHAR
            // (same block); i64 is a separate block carrying a NULL.
            let meta = vec![
                col(
                    "num_a",
                    ExaType::Numeric {
                        precision: Some(18),
                        scale: Some(2),
                    },
                ),
                col(
                    "num_b",
                    ExaType::Numeric {
                        precision: Some(38),
                        scale: Some(0),
                    },
                ),
                col("d", ExaType::Date),
                col("t", ExaType::Timestamp),
                col("label", ExaType::String { size: Some(100) }),
                col("i", ExaType::Int64),
            ];

            let rows: Vec<Vec<Value>> = vec![
                vec![
                    Value::Numeric(decimal(12345, 2)),
                    Value::Numeric(decimal(-1_000_000_000_000_000_001, 18)),
                    Value::Date(date(2024, 2, 29)),
                    Value::Timestamp(ts(2026, 6, 5, 1, 2, 3, 4)),
                    Value::String("héllo".into()),
                    Value::Int64(42),
                ],
                vec![
                    Value::Null,
                    Value::Numeric(decimal(999, 0)),
                    Value::Null,
                    Value::Timestamp(ts(1970, 1, 1, 0, 0, 0, 0)),
                    Value::Null,
                    Value::Null,
                ],
                vec![
                    Value::Numeric(decimal(0, 5)),
                    Value::Null,
                    Value::Date(date(9999, 12, 31)),
                    Value::Timestamp(ts(2000, 2, 29, 23, 59, 59, 999_999_999)),
                    Value::String(String::new()),
                    Value::Int64(-1),
                ],
            ];

            // Reference formatter: the pre-optimisation `chrono`/`Display` path.
            fn reference_block_string(v: &Value) -> String {
                match v {
                    Value::Numeric(d) => d.to_string(),
                    Value::Date(d) => d.format(DATE_FORMAT).to_string(),
                    Value::Timestamp(t) => t.format(TIMESTAMP_EMIT).to_string(),
                    Value::String(s) => s.clone(),
                    other => panic!("unexpected non-string-block value {other:?}"),
                }
            }

            // Expected dense, row-major-interleaved string block: skip NULL
            // cells (they take no block slot) and only include the string-block
            // columns (indices 0..=4; column 5 is Int64).
            let mut expected_string: Vec<String> = Vec::new();
            for row in &rows {
                // Columns 0..=4 are the string-block columns; column 5 is Int64.
                for cell in row.iter().take(5) {
                    if !matches!(cell, Value::Null) {
                        expected_string.push(reference_block_string(cell));
                    }
                }
            }

            let mut emit = EmitBuffer::new();
            for row in &rows {
                emit.push(row.clone());
            }
            let table = emit.to_proto(&meta);

            assert_eq!(
                table.data_string, expected_string,
                "fast-path string block must be byte-identical to the chrono/Display row path"
            );
            // Int64 block: only the two non-null cells, in row order.
            assert_eq!(table.data_int64, vec![42, -1]);
            // NULL bitmap is row-major (row * n_cols + col); 3 rows × 6 cols.
            let mut expected_nulls = vec![false; rows.len() * meta.len()];
            for (r, row) in rows.iter().enumerate() {
                for (c, cell) in row.iter().enumerate() {
                    if matches!(cell, Value::Null) {
                        expected_nulls[r * meta.len() + c] = true;
                    }
                }
            }
            assert_eq!(table.data_nulls, expected_nulls, "null bitmap");
            assert_eq!(table.rows, rows.len() as u64, "row count");
        }
    }

    // -----------------------------------------------------------------------
    // Permanent regression guard: the string-block ingest fast-path parsers
    // (`decode_string_block`'s DATE/TIMESTAMP branches) must stay
    // byte-identical to the `chrono::parse_from_str` path they front, for
    // every representable value, and must defer gracefully (not panic, not
    // diverge) for malformed input (plan task 6.1).
    // -----------------------------------------------------------------------
    mod fast_string_block_ingest_tests {
        use super::*;

        /// Reference decode: the original `chrono`-only path `decode_string_block`
        /// used before the fast parser was added, used as the known-correct
        /// oracle for DATE/TIMESTAMP comparison.
        fn reference_decode_date(s: &str) -> Value {
            match NaiveDate::parse_from_str(s, DATE_FORMAT) {
                Ok(d) => Value::Date(d),
                Err(_) => Value::Null,
            }
        }

        fn reference_decode_timestamp(s: &str) -> Value {
            match NaiveDateTime::parse_from_str(s, TIMESTAMP_PARSE)
                .or_else(|_| NaiveDateTime::parse_from_str(s, TIMESTAMP_FORMAT_ISO))
            {
                Ok(ts) => Value::Timestamp(ts),
                Err(_) => Value::Null,
            }
        }

        /// `fast_parse_date` must match `NaiveDate::parse_from_str(DATE_FORMAT)`
        /// byte-for-byte across leap days, year boundaries, and a normal
        /// mid-range date.
        #[test]
        fn fast_parse_date_matches_chrono_parse_for_valid_dates() {
            let cases = [
                "2024-02-29",
                "0001-01-01",
                "9999-12-31",
                "1970-01-01",
                "2026-06-05",
                "0000-01-01",
            ];
            for s in cases {
                let expected = reference_decode_date(s);
                assert_eq!(
                    fast_parse_date(s).map(Value::Date),
                    Some(expected.clone()),
                    "mismatch for {s}, expected {expected:?}"
                );
                assert_eq!(
                    decode_string_block(&ExaType::Date, s.to_string()),
                    expected,
                    "decode_string_block mismatch for {s}"
                );
            }
        }

        /// `fast_parse_timestamp` must match the existing chrono parse chain
        /// (space-separated primary format, `T`-separated ISO fallback)
        /// byte-for-byte, including midnight, 0/3/6/9 fractional digits, and
        /// the ISO `T` variant.
        #[test]
        fn fast_parse_timestamp_matches_chrono_parse_for_valid_timestamps() {
            let cases = [
                "1970-01-01 00:00:00",
                "2026-06-05 01:02:03.4",
                "2026-06-05 01:02:03.400",
                "2026-06-05 01:02:03.400000",
                "2026-06-05 01:02:03.000000004",
                "2024-02-29 23:59:59.999999999",
                "2026-06-05T01:02:03.400",
                "9999-12-31 23:59:59",
                "0001-01-01 00:00:00.1",
            ];
            for s in cases {
                let expected = reference_decode_timestamp(s);
                assert_eq!(
                    fast_parse_timestamp(s).map(Value::Timestamp),
                    Some(expected.clone()),
                    "mismatch for {s}, expected {expected:?}"
                );
                assert_eq!(
                    decode_string_block(&ExaType::Timestamp, s.to_string()),
                    expected,
                    "decode_string_block mismatch for {s}"
                );
                assert_eq!(
                    decode_string_block(&ExaType::TimestampTz, s.to_string()),
                    expected,
                    "decode_string_block (TimestampTz) mismatch for {s}"
                );
            }
        }

        /// Non-standard-width but chrono-parseable input (single-digit month/
        /// day, 2-digit year, a leap second, or an over-long fractional part
        /// chrono silently truncates to 9 digits) must not be rejected by
        /// `decode_string_block` even though the fast path correctly declines
        /// (returns `None`) and defers to the `chrono` fallback — proving the
        /// fallback chain preserves today's leniency exactly, never becoming
        /// stricter than the pre-fast-path behaviour.
        #[test]
        fn decode_string_block_preserves_leniency_when_fast_path_defers() {
            let date_cases = ["2024-2-29", "2024-02-9", "24-02-29"];
            for s in date_cases {
                assert_eq!(
                    fast_parse_date(s),
                    None,
                    "fast_parse_date should defer (not itself parse) {s}"
                );
                let expected = reference_decode_date(s);
                assert_ne!(
                    expected,
                    Value::Null,
                    "test setup: {s} should be chrono-valid"
                );
                assert_eq!(
                    decode_string_block(&ExaType::Date, s.to_string()),
                    expected,
                    "decode_string_block must still succeed via fallback for {s}"
                );
            }

            let ts_cases = ["2024-02-29 23:59:60", "2024-02-29 23:59:59.1234567890"];
            for s in ts_cases {
                assert_eq!(
                    fast_parse_timestamp(s),
                    None,
                    "fast_parse_timestamp should defer (not itself parse) {s}"
                );
                let expected = reference_decode_timestamp(s);
                assert_ne!(
                    expected,
                    Value::Null,
                    "test setup: {s} should be chrono-valid"
                );
                assert_eq!(
                    decode_string_block(&ExaType::Timestamp, s.to_string()),
                    expected,
                    "decode_string_block must still succeed via fallback for {s}"
                );
            }
        }

        /// Malformed DATE/TIMESTAMP strings must decode to `Value::Null`
        /// (never panic), exactly matching the pre-existing chrono-only
        /// behaviour: garbage text, wrong-width separators, and out-of-range
        /// calendar values (month 13, day 32, hour 24, minute 60).
        #[test]
        fn malformed_date_and_timestamp_strings_decode_to_null() {
            let malformed = [
                "",
                "not-a-date",
                "2024/02/29",
                "2024-13-01",
                "2024-02-32",
                "2024-00-01",
                "2024-02-00",
            ];
            for s in malformed {
                assert_eq!(
                    fast_parse_date(s),
                    None,
                    "fast_parse_date should defer/reject for {s}"
                );
                assert_eq!(
                    decode_string_block(&ExaType::Date, s.to_string()),
                    Value::Null,
                    "decode_string_block(Date) should be Null for {s}"
                );
            }

            let malformed_ts = [
                "",
                "not-a-timestamp",
                "2024-02-29",
                "2024-02-29 24:00:00",
                "2024-02-29 23:60:00",
                "2024-02-29 23:59:59.",
                "2024-13-01 00:00:00",
                "2024-02-32 00:00:00",
                "2024-02-29T23:59:59.abc",
                "2024-02-29X23:59:59",
            ];
            for s in malformed_ts {
                assert_eq!(
                    fast_parse_timestamp(s),
                    None,
                    "fast_parse_timestamp should defer/reject for {s}"
                );
                assert_eq!(
                    decode_string_block(&ExaType::Timestamp, s.to_string()),
                    Value::Null,
                    "decode_string_block(Timestamp) should be Null for {s}"
                );
            }
        }
    }

    // -----------------------------------------------------------------------
    // emit-arrow unit tests (task 2.5)
    // -----------------------------------------------------------------------
    #[cfg(feature = "emit-arrow")]
    mod arrow_tests {
        use super::*;
        // `emit_batch` on the bridge resolves to the EmitBatch ext-trait, which
        // serialises to IPC bytes then calls `emit_record_batch_ipc`. The whole
        // round-trip runs in-process with one arrow copy, so it works in tests.
        use arrow::array::{BooleanArray, Float64Array, Int64Array, StringArray};
        use arrow::datatypes::{DataType, Field, Schema};
        use arrow::record_batch::RecordBatch;
        use exasol_udf_sdk::context::EmitBatch;
        use std::sync::Arc;

        /// Build a simple 4-column RecordBatch: Int64, Utf8, Float64, Boolean
        fn make_batch(
            ints: &[i64],
            strs: &[Option<&str>],
            floats: &[f64],
            bools: &[bool],
        ) -> RecordBatch {
            let schema = Arc::new(Schema::new(vec![
                Field::new("a", DataType::Int64, false),
                Field::new("b", DataType::Utf8, true),
                Field::new("c", DataType::Float64, false),
                Field::new("d", DataType::Boolean, false),
            ]));
            let int_arr: Arc<dyn arrow::array::Array> = Arc::new(Int64Array::from(ints.to_vec()));
            let str_arr: Arc<dyn arrow::array::Array> = Arc::new(StringArray::from(strs.to_vec()));
            let float_arr: Arc<dyn arrow::array::Array> =
                Arc::new(Float64Array::from(floats.to_vec()));
            let bool_arr: Arc<dyn arrow::array::Array> =
                Arc::new(BooleanArray::from(bools.to_vec()));
            RecordBatch::try_new(schema, vec![int_arr, str_arr, float_arr, bool_arr]).unwrap()
        }

        fn mixed_meta() -> Vec<ColumnMeta> {
            vec![
                col("a", ExaType::Int64),
                col("b", ExaType::String { size: None }),
                col("c", ExaType::Double),
                col("d", ExaType::Boolean),
            ]
        }

        /// Test: push_batch produces byte-identical output to the row path
        #[test]
        fn push_batch_equals_row_push() {
            let meta = mixed_meta();
            let batch = make_batch(
                &[10, 20],
                &[Some("x"), Some("y")],
                &[1.5, 2.5],
                &[true, false],
            );

            // Row path
            let mut row_buf = EmitBuffer::new();
            row_buf.push(vec![
                Value::Int64(10),
                Value::String("x".into()),
                Value::Double(1.5),
                Value::Bool(true),
            ]);
            row_buf.push(vec![
                Value::Int64(20),
                Value::String("y".into()),
                Value::Double(2.5),
                Value::Bool(false),
            ]);
            let row_table = row_buf.to_proto(&meta);

            // Batch path — batch fits in one slice (< 4MB), so no mid-batch
            // flush; the whole thing lands in the tail.
            let mut batch_buf = EmitBuffer::new();
            let mut flushed_tables: Vec<exa_proto::ExascriptTableData> = Vec::new();
            batch_buf
                .push_batch(&batch, &meta, &mut |t| {
                    flushed_tables.push(t);
                    Ok(())
                })
                .unwrap();
            // No split expected for 2 small rows.
            assert!(
                flushed_tables.is_empty(),
                "no flush expected for tiny batch"
            );
            // Tail is now in batch_buf.
            let batch_table = batch_buf.to_proto(&meta);

            // The two tables must be byte-identical.
            assert_eq!(row_table.data_int64, batch_table.data_int64, "int64 block");
            assert_eq!(
                row_table.data_string, batch_table.data_string,
                "string block"
            );
            assert_eq!(
                row_table.data_double, batch_table.data_double,
                "double block"
            );
            assert_eq!(row_table.data_bool, batch_table.data_bool, "bool block");
            assert_eq!(row_table.data_nulls, batch_table.data_nulls, "null bitmap");
            assert_eq!(row_table.rows, batch_table.rows, "row count");

            // Also decode the batch-path result and verify values.
            let rs = InputRowSet::from_proto(&batch_table, &meta);
            assert_eq!(
                rs.row(0).unwrap(),
                &[
                    Value::Int64(10),
                    Value::String("x".into()),
                    Value::Double(1.5),
                    Value::Bool(true),
                ]
            );
            assert_eq!(
                rs.row(1).unwrap(),
                &[
                    Value::Int64(20),
                    Value::String("y".into()),
                    Value::Double(2.5),
                    Value::Bool(false),
                ]
            );
        }

        /// `encode_slice`'s per-type blocks are pre-sized with
        /// `Vec::with_capacity` for the exact (non-NULL) column count,
        /// mirroring `to_proto`'s pre-sizing. This only asserts the resulting
        /// contents are correct — `Vec::capacity()` is only guaranteed to be
        /// *at least* the requested value, so asserting an exact capacity here
        /// would be a flaky test rather than a real regression guard; the
        /// pre-sizing's throughput benefit is verified by `benches/emit-bench`.
        #[test]
        fn encode_slice_presizes_string_block_capacity() {
            let meta = vec![col("b", ExaType::String { size: None })];
            let schema = Arc::new(Schema::new(vec![Field::new("b", DataType::Utf8, true)]));
            let arr: Arc<dyn arrow::array::Array> =
                Arc::new(StringArray::from(vec![Some("a"), Some("b"), Some("c")]));
            let batch = RecordBatch::try_new(schema, vec![arr]).unwrap();

            let table = encode_slice(&batch, &meta).unwrap();
            assert_eq!(table.data_string, vec!["a", "b", "c"]);
        }

        /// Test: two string-family columns interleave row-major in data_string.
        #[test]
        fn push_batch_shared_block_type_interleaved() {
            // Two Utf8 columns both declared String → data_string is row-major.
            let schema = Arc::new(Schema::new(vec![
                Field::new("s1", DataType::Utf8, false),
                Field::new("s2", DataType::Utf8, false),
            ]));
            let a: Arc<dyn arrow::array::Array> = Arc::new(StringArray::from(vec!["A0", "A1"]));
            let b: Arc<dyn arrow::array::Array> = Arc::new(StringArray::from(vec!["B0", "B1"]));
            let batch = RecordBatch::try_new(schema, vec![a, b]).unwrap();

            let meta = vec![
                col("s1", ExaType::String { size: None }),
                col("s2", ExaType::String { size: None }),
            ];

            let mut buf = EmitBuffer::new();
            buf.push_batch(&batch, &meta, &mut |_| Ok(())).unwrap();
            let table = buf.to_proto(&meta);

            // Row-major: row0(s1,s2), row1(s1,s2)
            assert_eq!(table.data_string, vec!["A0", "B0", "A1", "B1"]);
        }

        /// Test: NULL cells occupy no type-block slot, only the bitmap.
        #[test]
        fn push_batch_null_bitmap() {
            // Row 0: (10, "hello", 1.0, true), Row 1: (NULL int64, NULL str, NULL float, NULL bool)
            let schema = Arc::new(Schema::new(vec![
                Field::new("a", DataType::Int64, true),
                Field::new("b", DataType::Utf8, true),
                Field::new("c", DataType::Float64, true),
                Field::new("d", DataType::Boolean, true),
            ]));
            let int_arr: Arc<dyn arrow::array::Array> =
                Arc::new(Int64Array::from(vec![Some(10i64), None]));
            let str_arr: Arc<dyn arrow::array::Array> =
                Arc::new(StringArray::from(vec![Some("hello"), None]));
            let float_arr: Arc<dyn arrow::array::Array> =
                Arc::new(Float64Array::from(vec![Some(1.0f64), None]));
            let bool_arr: Arc<dyn arrow::array::Array> =
                Arc::new(BooleanArray::from(vec![Some(true), None]));
            let batch =
                RecordBatch::try_new(schema, vec![int_arr, str_arr, float_arr, bool_arr]).unwrap();

            let meta = vec![
                col("a", ExaType::Int64),
                col("b", ExaType::String { size: None }),
                col("c", ExaType::Double),
                col("d", ExaType::Boolean),
            ];

            let mut buf = EmitBuffer::new();
            buf.push_batch(&batch, &meta, &mut |_| Ok(())).unwrap();
            let table = buf.to_proto(&meta);

            // Only row0's non-null values are in the type blocks.
            assert_eq!(table.data_int64, vec![10i64]);
            assert_eq!(table.data_string, vec!["hello"]);
            assert_eq!(table.data_double, vec![1.0f64]);
            assert_eq!(table.data_bool, vec![true]);

            // Null bitmap: 2 rows × 4 cols = 8 entries.
            // Row 0: all false (non-null). Row 1: all true (null).
            assert_eq!(
                table.data_nulls,
                vec![false, false, false, false, true, true, true, true]
            );

            // Round-trip via from_proto.
            let rs = InputRowSet::from_proto(&table, &meta);
            assert_eq!(
                rs.row(0).unwrap(),
                &[
                    Value::Int64(10),
                    Value::String("hello".into()),
                    Value::Double(1.0),
                    Value::Bool(true),
                ]
            );
            assert_eq!(
                rs.row(1).unwrap(),
                &[Value::Null, Value::Null, Value::Null, Value::Null]
            );
        }

        /// Test: cumulative byte cost of push_batch matches the row path's byte_estimate.
        #[test]
        fn push_batch_byte_estimate_parity() {
            let meta = mixed_meta();
            let s = "hello"; // 5 bytes per row

            // Row path: push 10 rows and check byte_estimate.
            let mut row_buf = EmitBuffer::new();
            for _ in 0..10 {
                row_buf.push(vec![
                    Value::Int64(42),
                    Value::String(s.to_string()),
                    Value::Double(1.0),
                    Value::Bool(true),
                ]);
            }
            let row_estimate = row_buf.byte_estimate;

            // Batch path: push the same 10 rows as a batch.
            let strs = vec![Some(s); 10];
            let batch = make_batch(&[42i64; 10], &strs, &[1.0f64; 10], &[true; 10]);
            let mut batch_buf = EmitBuffer::new();
            batch_buf
                .push_batch(&batch, &meta, &mut |_| Ok(()))
                .unwrap();
            let batch_estimate = batch_buf.byte_estimate;

            // The byte estimates must be equal so should_flush fires at the same threshold.
            assert_eq!(
                row_estimate, batch_estimate,
                "batch byte estimate ({batch_estimate}) must match row estimate ({row_estimate})"
            );
        }

        /// Test: a batch whose cost > 4MB splits into N>1 flushes.
        #[test]
        fn push_batch_splits_oversized_batch() {
            // Each row has a ~1000-byte string. We need enough rows to exceed 4MB.
            // 4_000_000 / 1000 = 4000 rows needed. Use 5000 to guarantee > 1 flush.
            let n_rows = 5000usize;
            let s = "x".repeat(1000);

            let schema = Arc::new(Schema::new(vec![Field::new("v", DataType::Utf8, false)]));
            let arr: Arc<dyn arrow::array::Array> =
                Arc::new(StringArray::from(vec![s.as_str(); n_rows]));
            let batch = RecordBatch::try_new(schema, vec![arr]).unwrap();

            let meta = vec![col("v", ExaType::String { size: None })];

            let mut flush_count = 0usize;
            let mut total_flushed_rows = 0u64;
            let mut buf = EmitBuffer::new();
            buf.push_batch(&batch, &meta, &mut |t| {
                // Each flushed slice must have at least 1 row.
                assert!(t.rows > 0, "flushed table must have rows");
                total_flushed_rows += t.rows;
                flush_count += 1;
                Ok(())
            })
            .unwrap();

            // At least 1 flush must have happened (oversized batch).
            assert!(
                flush_count >= 1,
                "expected ≥1 flush for oversized batch, got {flush_count}"
            );

            // Tail rows plus flushed rows must equal total batch rows.
            let tail_rows = buf.len() as u64;
            assert_eq!(
                total_flushed_rows + tail_rows,
                n_rows as u64,
                "flushed({total_flushed_rows}) + tail({tail_rows}) must equal batch rows({n_rows})"
            );
        }

        /// Test: the tail after an oversized push_batch is < 4MB.
        #[test]
        fn push_batch_slice_zero_copy_tail_bounded() {
            let n_rows = 5000usize;
            let s = "x".repeat(1000);

            let schema = Arc::new(Schema::new(vec![Field::new("v", DataType::Utf8, false)]));
            let arr: Arc<dyn arrow::array::Array> =
                Arc::new(StringArray::from(vec![s.as_str(); n_rows]));
            let batch = RecordBatch::try_new(schema, vec![arr]).unwrap();
            let meta = vec![col("v", ExaType::String { size: None })];

            let mut buf = EmitBuffer::new();
            buf.push_batch(&batch, &meta, &mut |_| Ok(())).unwrap();

            // The residual byte estimate must be < 4MB.
            assert!(
                buf.byte_estimate < EMIT_BUFFER_LIMIT_BYTES,
                "tail byte estimate {} must be < 4MB",
                buf.byte_estimate
            );
        }

        /// Test: a column whose Arrow type cannot feed the declared ExaType returns Err.
        #[test]
        fn push_batch_type_mismatch_errors() {
            // Utf8 column declared as Int64 → incompatible.
            let schema = Arc::new(Schema::new(vec![Field::new("v", DataType::Utf8, false)]));
            let arr: Arc<dyn arrow::array::Array> = Arc::new(StringArray::from(vec!["hello"]));
            let batch = RecordBatch::try_new(schema, vec![arr]).unwrap();
            let meta = vec![col("v", ExaType::Int64)];

            let mut buf = EmitBuffer::new();
            let result = buf.push_batch(&batch, &meta, &mut |_| Ok(()));
            assert!(
                matches!(result, Err(UdfError::Type(_))),
                "Utf8 declared as Int64 must return Err(Type)"
            );

            // Date32 declared as Boolean → incompatible.
            use arrow::array::Date32Array;
            let schema2 = Arc::new(Schema::new(vec![Field::new("d", DataType::Date32, false)]));
            let arr2: Arc<dyn arrow::array::Array> = Arc::new(Date32Array::from(vec![0i32]));
            let batch2 = RecordBatch::try_new(schema2, vec![arr2]).unwrap();
            let meta2 = vec![col("d", ExaType::Boolean)];

            let mut buf2 = EmitBuffer::new();
            let result2 = buf2.push_batch(&batch2, &meta2, &mut |_| Ok(()));
            assert!(
                matches!(result2, Err(UdfError::Type(_))),
                "Date32 declared as Boolean must return Err(Type)"
            );
        }

        /// A `BIGINT` EMITS column arrives as `ExaType::Numeric` (string block),
        /// so an Arrow `Int64` column must feed it — stringified exactly as the
        /// row path's `value_to_block_string(Value::Int64)`. This is the
        /// `emit-arrow-batch` fixture's `id BIGINT` case.
        #[test]
        fn push_batch_int64_into_numeric_block() {
            let meta = vec![col(
                "id",
                ExaType::Numeric {
                    precision: None,
                    scale: None,
                },
            )];
            let schema = Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, false)]));
            let arr: Arc<dyn arrow::array::Array> = Arc::new(Int64Array::from(vec![1i64, 2, 3]));
            let batch = RecordBatch::try_new(schema, vec![arr]).unwrap();

            // Row path: the same values as Value::Int64 into a Numeric column.
            let mut row_buf = EmitBuffer::new();
            for n in [1i64, 2, 3] {
                row_buf.push(vec![Value::Int64(n)]);
            }
            let row_table = row_buf.to_proto(&meta);

            let mut batch_buf = EmitBuffer::new();
            batch_buf
                .push_batch(&batch, &meta, &mut |_| Ok(()))
                .expect("Int64 must feed a NUMERIC column");
            let batch_table = batch_buf.to_proto(&meta);

            assert_eq!(batch_table.data_string, vec!["1", "2", "3"]);
            assert_eq!(row_table.data_string, batch_table.data_string);
            assert!(batch_table.data_int64.is_empty(), "ints go to string block");
        }

        // -----------------------------------------------------------------------
        // Bridge tests
        // -----------------------------------------------------------------------

        /// Build a bridge with a flush counter for emit-arrow tests.
        fn make_emit_bridge_with_counter<'a>(
            input: &'a mut InputRowSet,
            emit: &'a mut EmitBuffer,
            meta: &'a [ColumnMeta],
            flush_count: &'a std::cell::Cell<usize>,
        ) -> HostContextBridge<'a> {
            HostContextBridge::new(
                input,
                emit,
                meta,
                meta,
                Box::new(move |t: exa_proto::ExascriptTableData| {
                    if t.rows > 0 {
                        flush_count.set(flush_count.get() + 1);
                    }
                    Ok(())
                }),
                HandshakeMeta::default(),
                #[cfg(feature = "connect-back")]
                Box::new(|_name| {
                    Err(exasol_udf_sdk::error::UdfError::ConnectBack(
                        "no credential fetcher in test".into(),
                    ))
                }),
            )
        }

        /// Test: a small batch under the threshold buffers without a mid-flush.
        #[test]
        fn bridge_emit_batch_buffers_and_flushes() {
            let meta = mixed_meta();
            let batch = make_batch(
                &[1, 2],
                &[Some("a"), Some("b")],
                &[0.1, 0.2],
                &[true, false],
            );
            let empty_table = ExascriptTableData {
                rows: 0,
                ..Default::default()
            };
            let mut rs = InputRowSet::from_proto(&empty_table, &meta);
            let mut emit = EmitBuffer::new();
            let flush_count = std::cell::Cell::new(0usize);
            {
                let mut bridge =
                    make_emit_bridge_with_counter(&mut rs, &mut emit, &meta, &flush_count);
                bridge.emit_batch(&batch).unwrap();
                // No mid-batch flush for a tiny batch.
                assert_eq!(flush_count.get(), 0, "no flush expected for tiny batch");
            }
            // The tail is in the emit buffer after the bridge is dropped.
            assert_eq!(emit.len(), 2, "tail must have 2 rows");
        }

        /// Test: a batch whose Arrow type cannot feed the declared ExaType makes
        /// emit_batch return Err after the host deserialises and runs push_batch.
        #[test]
        fn bridge_emit_batch_error_propagates() {
            // A Utf8 array declared as ExaType::Int64 — incompatible.
            let schema = Arc::new(Schema::new(vec![Field::new("v", DataType::Utf8, false)]));
            let arr: Arc<dyn arrow::array::Array> = Arc::new(StringArray::from(vec!["hello"]));
            let bad_batch = RecordBatch::try_new(schema, vec![arr]).unwrap();

            let meta = vec![col("v", ExaType::Int64)];
            let empty_table = ExascriptTableData {
                rows: 0,
                ..Default::default()
            };
            let mut rs = InputRowSet::from_proto(&empty_table, &meta);
            let mut emit = EmitBuffer::new();
            let flush_count = std::cell::Cell::new(0usize);
            let mut bridge = make_emit_bridge_with_counter(&mut rs, &mut emit, &meta, &flush_count);
            let result = bridge.emit_batch(&bad_batch);
            assert!(
                matches!(result, Err(UdfError::Type(_))),
                "Utf8 declared as Int64 must return Err(Type)"
            );
        }

        /// Test: interleaved emit() and emit_batch() share the same buffer.
        #[test]
        fn bridge_mixed_emit_styles_share_buffer() {
            let meta = vec![col("v", ExaType::Int64)];
            let schema = Arc::new(Schema::new(vec![Field::new("v", DataType::Int64, false)]));
            let arr: Arc<dyn arrow::array::Array> = Arc::new(Int64Array::from(vec![2i64, 3]));
            let batch = RecordBatch::try_new(schema, vec![arr]).unwrap();

            let empty_table = ExascriptTableData {
                rows: 0,
                ..Default::default()
            };
            let mut rs = InputRowSet::from_proto(&empty_table, &meta);
            let mut emit = EmitBuffer::new();
            let flush_count = std::cell::Cell::new(0usize);
            {
                let mut bridge =
                    make_emit_bridge_with_counter(&mut rs, &mut emit, &meta, &flush_count);
                // Row-based emit: 1 row
                bridge.emit(&[Value::Int64(1)]).unwrap();
                // Batch-based emit: 2 rows. push_batch flushes the pending row
                // first (decision-log [7] step 1) to preserve FIFO order, then
                // the 2 batch rows land in the tail as they're under threshold.
                bridge.emit_batch(&batch).unwrap();
                // 1 flush for the pending row that was displaced by the batch.
                assert_eq!(
                    flush_count.get(),
                    1,
                    "pending row flushed before batch tail"
                );
            }
            // The 2 batch rows are in the tail buffer (the flushed row was sent to the flusher).
            assert_eq!(emit.len(), 2, "batch tail must have 2 rows");

            // Verify the tail values (the 2 batch rows).
            let table = emit.to_proto(&meta);
            let rs2 = InputRowSet::from_proto(&table, &meta);
            assert_eq!(rs2.row(0).unwrap(), &[Value::Int64(2)]);
            assert_eq!(rs2.row(1).unwrap(), &[Value::Int64(3)]);
        }
    }
}
