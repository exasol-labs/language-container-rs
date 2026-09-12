//! Benchmark UDF entry points, one cdylib for both benchmark tiers.
//!
//! Tier 1 (`exa-udf-runtime/benches/protocol.rs`) loads this library through
//! the mock engine; Tier 2 (`benches/udf-bench`) uploads it to BucketFS and
//! registers one script per entry point. See `benches/README.md`.
//!
//! Four column classes, shared by both tiers:
//!
//! | class      | columns                                              |
//! |------------|------------------------------------------------------|
//! | `native`   | `k DECIMAL(18,0), v DOUBLE`                          |
//! | `strblock` | `k DECIMAL(18,0), amount DECIMAL(18,2), d DATE, ts TIMESTAMP` |
//! | `varchar`  | `k DECIMAL(18,0), label VARCHAR(100)`                |
//! | `wide`     | 24 columns, see [`WIDE_COLUMNS`]: emit-only, models a UDF that expands one input row (a file reference) into millions of wide rows |
//!
//! Every entry point reads all of its input columns so ingest decode is never
//! skipped. Generators take `(n, do_emit)`: with `do_emit = 0` they build every
//! row and emit one sentinel, so transfer is full minus generation. The wide
//! batch generator takes a third `batch_rows` parameter, the rows per Arrow
//! record batch, the knob a file-reading UDF actually controls.

use std::sync::Arc;

use arrow::array::{
    ArrayRef, BooleanArray, Date32Array, Decimal128Array, Float64Array, Int64Array, StringArray,
    TimestampNanosecondArray,
};
use arrow::datatypes::{DataType, Field, Schema, TimeUnit};
use arrow::record_batch::RecordBatch;
use chrono::{NaiveDate, NaiveDateTime};
use exasol_udf_macros::exasol_udf;
use exasol_udf_sdk::context::{EmitBatch, UdfContext};
use exasol_udf_sdk::error::UdfError;
use exasol_udf_sdk::value::{Decimal, Value};

/// Byte length of every `label` value.
pub const LABEL_LEN: usize = 50;

/// Rows per Arrow record batch in the `_batch` entry points. The host re-splits
/// at the 4,000,000-byte `MT_EMIT` limit anyway; this bounds peak UDF-side memory.
pub const CHUNK: usize = 100_000;

/// `amount` scale, matching `DECIMAL(18,2)`.
const AMOUNT_SCALE: u8 = 2;

/// Scale of the wide class's `big*` columns, matching `DECIMAL(36,10)`.
const BIG_SCALE: u8 = 10;

pub use bench_schema::{WIDE_BATCH_ROWS, WIDE_COLUMNS, wide_ddl};

/// A nullable wide column is NULL in one row out of this many.
pub const NULL_EVERY: i64 = 10;

/// Maximum length of the wide VARCHAR column at `col` (16..24).
const WIDE_TEXT_MAX: [usize; 8] = [8, 16, 32, 32, 64, 64, 128, 200];

/// Source text for the wide VARCHAR columns; slices of it are the values.
const WIDE_TEXT: &str = "the quick brown fox jumps over the lazy dog while forty-two     benchmark rows stream through a script language container, each carrying a     reference to a file whose contents are emitted as wide rows with dozens of     columns of mixed types and a sprinkling of nulls";

/// Whether wide column `col` is NULL in row `i`.
#[inline]
pub fn wide_is_null(i: i64, col: usize) -> bool {
    WIDE_COLUMNS[col].2 && (i + col as i64) % NULL_EVERY == 0
}

/// Text for wide VARCHAR column `col` (16..24) in row `i`: length cycles over
/// `1..=max`, content is a window into [`WIDE_TEXT`], so values repeat the way
/// file data does and length varies the way free text does.
#[inline]
pub fn wide_text(i: i64, col: usize) -> String {
    let max = WIDE_TEXT_MAX[col - 16];
    let len = 1 + ((i as usize) * 31 + col * 7) % max;
    let start = (i as usize * 13 + col) % (WIDE_TEXT.len() - len);
    WIDE_TEXT[start..start + len].to_string()
}

/// `big*` for row `i`: a `DECIMAL(36,10)` with a full ten-digit fraction.
#[inline]
pub fn big(i: i64) -> Decimal {
    Decimal {
        unscaled: (i as i128) * 12_345_678_901 + 987_654_321,
        scale: BIG_SCALE,
    }
}

// ---------------------------------------------------------------------------
// Row generators. The row and batch entry points share these so both modes pay
// the same per-row construction cost.
// ---------------------------------------------------------------------------

/// `v` for row `i`.
#[inline]
pub fn native_v(i: i64) -> f64 {
    i as f64 * 1.5
}

/// `amount` for row `i`: a `DECIMAL(18,2)` growing with `i`.
#[inline]
pub fn amount(i: i64) -> Decimal {
    Decimal {
        unscaled: (i as i128) * 137 + 4200,
        scale: AMOUNT_SCALE,
    }
}

fn base_date() -> NaiveDate {
    NaiveDate::from_ymd_opt(2020, 1, 1).expect("valid base date")
}

/// `d` for row `i`: cycles through a ten-year window so date formatting sees
/// varying output.
#[inline]
pub fn date(i: i64) -> NaiveDate {
    base_date() + chrono::Duration::days(i % 3650)
}

/// `ts` for row `i`: seconds advance with `i`; microseconds cycle so the
/// fractional-second path is exercised. Microsecond granularity because that is
/// what the database delivers for TIMESTAMP inputs.
#[inline]
pub fn timestamp(i: i64) -> NaiveDateTime {
    let micros = (i * 137).rem_euclid(1_000_000);
    base_date().and_hms_opt(0, 0, 0).expect("midnight")
        + chrono::Duration::seconds(i)
        + chrono::Duration::microseconds(micros)
}

/// `label` for row `i`: `i` left-padded with zeros to [`LABEL_LEN`] bytes.
#[inline]
pub fn label(i: i64) -> String {
    format!("{i:0>LABEL_LEN$}")
}

#[inline]
pub fn native_row(i: i64) -> [Value; 2] {
    [Value::Int64(i), Value::Double(native_v(i))]
}

#[inline]
pub fn strblock_row(i: i64) -> [Value; 4] {
    [
        Value::Int64(i),
        Value::Numeric(amount(i)),
        Value::Date(date(i)),
        Value::Timestamp(timestamp(i)),
    ]
}

#[inline]
pub fn varchar_row(i: i64) -> [Value; 2] {
    [Value::Int64(i), Value::String(label(i))]
}

/// One wide row, see [`WIDE_COLUMNS`].
pub fn wide_row(i: i64) -> Vec<Value> {
    let v = |col: usize, value: Value| {
        if wide_is_null(i, col) {
            Value::Null
        } else {
            value
        }
    };
    vec![
        Value::Int64(i),
        v(1, Value::Int64(i * 7)),
        Value::Int64(i % 1_000),
        v(3, Value::Double(native_v(i))),
        Value::Double(i as f64 / 3.0),
        v(5, Value::Double(-(i as f64) * 0.25)),
        Value::Bool(i % 2 == 0),
        v(7, Value::Bool(i % 3 == 0)),
        Value::Numeric(amount(i)),
        v(9, Value::Numeric(amount(i + 1))),
        Value::Numeric(big(i)),
        v(11, Value::Numeric(big(i + 1))),
        Value::Date(date(i)),
        v(13, Value::Date(date(i + 1))),
        Value::Timestamp(timestamp(i)),
        v(15, Value::Timestamp(timestamp(i + 1))),
        Value::String(format!("{i:08x}")),
        v(17, Value::String(wide_text(i, 17))),
        Value::String(wide_text(i, 18)),
        v(19, Value::String(wide_text(i, 19))),
        Value::String(wide_text(i, 20)),
        v(21, Value::String(wide_text(i, 21))),
        Value::String(wide_text(i, 22)),
        v(23, Value::String(wide_text(i, 23))),
    ]
}

// ---------------------------------------------------------------------------
// Arrow batch builders, one per class.
// ---------------------------------------------------------------------------

fn date_to_epoch_days(d: NaiveDate) -> i32 {
    let epoch = NaiveDate::from_ymd_opt(1970, 1, 1).expect("epoch");
    d.signed_duration_since(epoch).num_days() as i32
}

fn ts_to_epoch_nanos(ts: NaiveDateTime) -> i64 {
    ts.and_utc()
        .timestamp_nanos_opt()
        .expect("timestamp within the nanosecond range")
}

fn arrow_err(e: impl std::fmt::Display) -> UdfError {
    UdfError::User(e.to_string())
}

pub fn native_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("k", DataType::Int64, false),
        Field::new("v", DataType::Float64, false),
    ]))
}

pub fn strblock_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("k", DataType::Int64, false),
        Field::new("amount", DataType::Decimal128(18, 2), false),
        Field::new("d", DataType::Date32, false),
        Field::new("ts", DataType::Timestamp(TimeUnit::Nanosecond, None), false),
    ]))
}

pub fn varchar_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("k", DataType::Int64, false),
        Field::new("label", DataType::Utf8, false),
    ]))
}

pub fn wide_schema() -> Arc<Schema> {
    let fields = WIDE_COLUMNS
        .iter()
        .map(|(name, ty, nullable)| {
            let dt = match *ty {
                "DECIMAL(18,0)" => DataType::Int64,
                "DOUBLE" => DataType::Float64,
                "BOOLEAN" => DataType::Boolean,
                "DECIMAL(18,2)" => DataType::Decimal128(18, AMOUNT_SCALE as i8),
                "DECIMAL(36,10)" => DataType::Decimal128(36, BIG_SCALE as i8),
                "DATE" => DataType::Date32,
                "TIMESTAMP" => DataType::Timestamp(TimeUnit::Nanosecond, None),
                _ => DataType::Utf8,
            };
            Field::new(*name, dt, *nullable)
        })
        .collect::<Vec<_>>();
    Arc::new(Schema::new(fields))
}

/// `len` wide rows starting at `k = start`, built column by column from the
/// same per-cell generators as [`wide_row`].
pub fn wide_batch(start: i64, len: usize) -> Result<RecordBatch, UdfError> {
    let ks = start..start + len as i64;
    let opt = |col: usize, f: &dyn Fn(i64) -> Value| -> Vec<Option<Value>> {
        ks.clone()
            .map(|i| {
                if wide_is_null(i, col) {
                    None
                } else {
                    Some(f(i))
                }
            })
            .collect()
    };
    let mut cols: Vec<ArrayRef> = Vec::with_capacity(WIDE_COLUMNS.len());
    for (col, (_, ty, _)) in WIDE_COLUMNS.iter().enumerate() {
        let row = wide_row_cell(col);
        let values = opt(col, &row);
        let arr: ArrayRef = match *ty {
            "DECIMAL(18,0)" => Arc::new(Int64Array::from(
                values
                    .iter()
                    .map(|v| match v {
                        Some(Value::Int64(x)) => Some(*x),
                        _ => None,
                    })
                    .collect::<Vec<_>>(),
            )),
            "DOUBLE" => Arc::new(Float64Array::from(
                values
                    .iter()
                    .map(|v| match v {
                        Some(Value::Double(x)) => Some(*x),
                        _ => None,
                    })
                    .collect::<Vec<_>>(),
            )),
            "BOOLEAN" => Arc::new(BooleanArray::from(
                values
                    .iter()
                    .map(|v| match v {
                        Some(Value::Bool(x)) => Some(*x),
                        _ => None,
                    })
                    .collect::<Vec<_>>(),
            )),
            "DECIMAL(18,2)" | "DECIMAL(36,10)" => {
                let (p, s) = if *ty == "DECIMAL(18,2)" {
                    (18, AMOUNT_SCALE)
                } else {
                    (36, BIG_SCALE)
                };
                Arc::new(
                    Decimal128Array::from(
                        values
                            .iter()
                            .map(|v| match v {
                                Some(Value::Numeric(d)) => Some(d.unscaled),
                                _ => None,
                            })
                            .collect::<Vec<_>>(),
                    )
                    .with_precision_and_scale(p, s as i8)
                    .map_err(arrow_err)?,
                )
            }
            "DATE" => Arc::new(Date32Array::from(
                values
                    .iter()
                    .map(|v| match v {
                        Some(Value::Date(d)) => Some(date_to_epoch_days(*d)),
                        _ => None,
                    })
                    .collect::<Vec<_>>(),
            )),
            "TIMESTAMP" => Arc::new(TimestampNanosecondArray::from(
                values
                    .iter()
                    .map(|v| match v {
                        Some(Value::Timestamp(t)) => Some(ts_to_epoch_nanos(*t)),
                        _ => None,
                    })
                    .collect::<Vec<_>>(),
            )),
            _ => Arc::new(StringArray::from(
                values
                    .iter()
                    .map(|v| match v {
                        Some(Value::String(s)) => Some(s.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>(),
            )),
        };
        cols.push(arr);
    }
    RecordBatch::try_new(wide_schema(), cols).map_err(arrow_err)
}

/// The non-NULL value of wide column `col` in row `i`, as [`wide_row`] builds it.
fn wide_row_cell(col: usize) -> impl Fn(i64) -> Value {
    move |i| match col {
        0 => Value::Int64(i),
        1 => Value::Int64(i * 7),
        2 => Value::Int64(i % 1_000),
        3 => Value::Double(native_v(i)),
        4 => Value::Double(i as f64 / 3.0),
        5 => Value::Double(-(i as f64) * 0.25),
        6 => Value::Bool(i % 2 == 0),
        7 => Value::Bool(i % 3 == 0),
        8 => Value::Numeric(amount(i)),
        9 => Value::Numeric(amount(i + 1)),
        10 => Value::Numeric(big(i)),
        11 => Value::Numeric(big(i + 1)),
        12 => Value::Date(date(i)),
        13 => Value::Date(date(i + 1)),
        14 => Value::Timestamp(timestamp(i)),
        15 => Value::Timestamp(timestamp(i + 1)),
        16 => Value::String(format!("{i:08x}")),
        _ => Value::String(wide_text(i, col)),
    }
}

/// `len` native rows starting at `k = start`.
pub fn native_batch(start: i64, len: usize) -> Result<RecordBatch, UdfError> {
    let ks: Vec<i64> = (start..start + len as i64).collect();
    let vs: Vec<f64> = ks.iter().map(|&i| native_v(i)).collect();
    RecordBatch::try_new(
        native_schema(),
        vec![
            Arc::new(Int64Array::from(ks)),
            Arc::new(Float64Array::from(vs)),
        ],
    )
    .map_err(arrow_err)
}

/// `len` strblock rows starting at `k = start`.
pub fn strblock_batch(start: i64, len: usize) -> Result<RecordBatch, UdfError> {
    let ks: Vec<i64> = (start..start + len as i64).collect();
    let amounts: Vec<i128> = ks.iter().map(|&i| amount(i).unscaled).collect();
    let dates: Vec<i32> = ks.iter().map(|&i| date_to_epoch_days(date(i))).collect();
    let tss: Vec<i64> = ks
        .iter()
        .map(|&i| ts_to_epoch_nanos(timestamp(i)))
        .collect();
    let amounts = Decimal128Array::from(amounts)
        .with_precision_and_scale(18, AMOUNT_SCALE as i8)
        .map_err(arrow_err)?;
    RecordBatch::try_new(
        strblock_schema(),
        vec![
            Arc::new(Int64Array::from(ks)),
            Arc::new(amounts),
            Arc::new(Date32Array::from(dates)),
            Arc::new(TimestampNanosecondArray::from(tss)),
        ],
    )
    .map_err(arrow_err)
}

/// `len` varchar rows starting at `k = start`.
pub fn varchar_batch(start: i64, len: usize) -> Result<RecordBatch, UdfError> {
    let ks: Vec<i64> = (start..start + len as i64).collect();
    let labels: Vec<String> = ks.iter().map(|&i| label(i)).collect();
    RecordBatch::try_new(
        varchar_schema(),
        vec![
            Arc::new(Int64Array::from(ks)),
            Arc::new(StringArray::from(labels)),
        ],
    )
    .map_err(arrow_err)
}

// ---------------------------------------------------------------------------
// Shared drivers.
// ---------------------------------------------------------------------------

/// Read `(n, do_emit)` from the current row without advancing (SCALAR input).
fn scalar_params(ctx: &dyn UdfContext) -> Result<(i64, bool), UdfError> {
    let n = ctx
        .get_i64(0)?
        .ok_or_else(|| UdfError::User("bench-udfs: n is NULL".into()))?;
    let do_emit = ctx.get_i64(1)?.unwrap_or(0) != 0;
    Ok((n, do_emit))
}

/// Rows per record batch from an optional third `batch_rows` column, else
/// [`CHUNK`].
fn batch_rows(ctx: &dyn UdfContext) -> Result<usize, UdfError> {
    if ctx.num_columns() < 3 {
        return Ok(CHUNK);
    }
    match ctx.get_i64(2)? {
        Some(b) if b > 0 => Ok(b as usize),
        _ => Ok(CHUNK),
    }
}

/// Read `(n, do_emit)` from the first row of a SET group, then drain the group.
fn set_params(ctx: &mut dyn UdfContext) -> Result<(i64, bool), UdfError> {
    if !ctx.next()? {
        return Err(UdfError::User("bench-udfs: empty input group".into()));
    }
    let params = scalar_params(ctx)?;
    while ctx.next()? {}
    Ok(params)
}

/// Emit `n` rows one at a time, or build them all and emit a single sentinel.
fn generate_rows<R: AsRef<[Value]>>(
    ctx: &mut dyn UdfContext,
    n: i64,
    do_emit: bool,
    row: impl Fn(i64) -> R,
) -> Result<(), UdfError> {
    if do_emit {
        for i in 0..n {
            ctx.emit(row(i).as_ref())?;
        }
    } else {
        for i in 0..n {
            std::hint::black_box(row(i));
        }
        ctx.emit(row(0).as_ref())?;
    }
    Ok(())
}

/// Emit `n` rows as `chunk`-row record batches, or build them all and emit a
/// single one-row sentinel batch.
fn generate_batches(
    ctx: &mut dyn UdfContext,
    n: i64,
    do_emit: bool,
    chunk: usize,
    batch: impl Fn(i64, usize) -> Result<RecordBatch, UdfError>,
) -> Result<(), UdfError> {
    let mut start = 0i64;
    while start < n {
        let len = ((n - start) as usize).min(chunk.max(1));
        let b = batch(start, len)?;
        if do_emit {
            ctx.emit_batch(&b)?;
        } else {
            std::hint::black_box(&b);
        }
        start += len as i64;
    }
    if !do_emit {
        ctx.emit_batch(&batch(0, 1)?)?;
    }
    Ok(())
}

/// Clone the current input row.
fn current_row(ctx: &dyn UdfContext) -> Result<Vec<Value>, UdfError> {
    (0..ctx.num_columns())
        .map(|c| ctx.get(c).cloned())
        .collect()
}

/// Re-emit every input row of the group, one `ctx.emit` per row.
fn reemit_rows(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    while ctx.next()? {
        let row = current_row(ctx)?;
        ctx.emit(&row)?;
    }
    Ok(())
}

/// Accumulates typed input cells and turns them into record batches.
trait ColumnCollector: Default {
    fn push(&mut self, ctx: &dyn UdfContext) -> Result<(), UdfError>;
    fn len(&self) -> usize;
    fn take_batch(&mut self) -> Result<RecordBatch, UdfError>;
}

#[derive(Default)]
struct NativeCollector {
    k: Vec<i64>,
    v: Vec<f64>,
}

impl ColumnCollector for NativeCollector {
    fn push(&mut self, ctx: &dyn UdfContext) -> Result<(), UdfError> {
        self.k.push(ctx.get_i64(0)?.unwrap_or(0));
        self.v.push(ctx.get_f64(1)?.unwrap_or(0.0));
        Ok(())
    }
    fn len(&self) -> usize {
        self.k.len()
    }
    fn take_batch(&mut self) -> Result<RecordBatch, UdfError> {
        RecordBatch::try_new(
            native_schema(),
            vec![
                Arc::new(Int64Array::from(std::mem::take(&mut self.k))),
                Arc::new(Float64Array::from(std::mem::take(&mut self.v))),
            ],
        )
        .map_err(arrow_err)
    }
}

#[derive(Default)]
struct StrblockCollector {
    k: Vec<i64>,
    amount: Vec<i128>,
    d: Vec<i32>,
    ts: Vec<i64>,
}

impl ColumnCollector for StrblockCollector {
    fn push(&mut self, ctx: &dyn UdfContext) -> Result<(), UdfError> {
        self.k.push(ctx.get_i64(0)?.unwrap_or(0));
        self.amount
            .push(ctx.get_decimal(1)?.map(|d| d.unscaled).unwrap_or(0));
        self.d
            .push(ctx.get_date(2)?.map(date_to_epoch_days).unwrap_or(0));
        self.ts
            .push(ctx.get_timestamp(3)?.map(ts_to_epoch_nanos).unwrap_or(0));
        Ok(())
    }
    fn len(&self) -> usize {
        self.k.len()
    }
    fn take_batch(&mut self) -> Result<RecordBatch, UdfError> {
        let amounts = Decimal128Array::from(std::mem::take(&mut self.amount))
            .with_precision_and_scale(18, AMOUNT_SCALE as i8)
            .map_err(arrow_err)?;
        RecordBatch::try_new(
            strblock_schema(),
            vec![
                Arc::new(Int64Array::from(std::mem::take(&mut self.k))),
                Arc::new(amounts),
                Arc::new(Date32Array::from(std::mem::take(&mut self.d))),
                Arc::new(TimestampNanosecondArray::from(std::mem::take(&mut self.ts))),
            ],
        )
        .map_err(arrow_err)
    }
}

#[derive(Default)]
struct VarcharCollector {
    k: Vec<i64>,
    label: Vec<String>,
}

impl ColumnCollector for VarcharCollector {
    fn push(&mut self, ctx: &dyn UdfContext) -> Result<(), UdfError> {
        self.k.push(ctx.get_i64(0)?.unwrap_or(0));
        self.label
            .push(ctx.get_string(1)?.unwrap_or_default().to_string());
        Ok(())
    }
    fn len(&self) -> usize {
        self.k.len()
    }
    fn take_batch(&mut self) -> Result<RecordBatch, UdfError> {
        RecordBatch::try_new(
            varchar_schema(),
            vec![
                Arc::new(Int64Array::from(std::mem::take(&mut self.k))),
                Arc::new(StringArray::from(std::mem::take(&mut self.label))),
            ],
        )
        .map_err(arrow_err)
    }
}

/// Re-emit every input row of the group in [`CHUNK`]-row record batches.
fn reemit_batches<C: ColumnCollector>(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    let mut col = C::default();
    while ctx.next()? {
        col.push(ctx)?;
        if col.len() == CHUNK {
            let b = col.take_batch()?;
            ctx.emit_batch(&b)?;
        }
    }
    if col.len() > 0 {
        let b = col.take_batch()?;
        ctx.emit_batch(&b)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// SCALAR RETURNS: sr_<class>
// ---------------------------------------------------------------------------

/// `(k, v) RETURNS DECIMAL(18,0)`: `k + 1`.
#[exasol_udf]
pub fn sr_native(ctx: &mut dyn UdfContext) -> Result<Option<i64>, UdfError> {
    let k = ctx.get_i64(0)?;
    std::hint::black_box(ctx.get_f64(1)?);
    Ok(k.map(|k| k + 1))
}

/// `(k, amount, d, ts) RETURNS DECIMAL(18,2)`: `amount * 2`.
#[exasol_udf]
pub fn sr_strblock(ctx: &mut dyn UdfContext) -> Result<Option<Decimal>, UdfError> {
    std::hint::black_box(ctx.get_i64(0)?);
    let amount = ctx.get_decimal(1)?;
    std::hint::black_box(ctx.get_date(2)?);
    std::hint::black_box(ctx.get_timestamp(3)?);
    Ok(amount.map(|a| Decimal {
        unscaled: a.unscaled * 2,
        scale: a.scale,
    }))
}

/// `(k, label) RETURNS DECIMAL(18,0)`: byte length of `label`.
#[exasol_udf]
pub fn sr_varchar(ctx: &mut dyn UdfContext) -> Result<Option<i64>, UdfError> {
    std::hint::black_box(ctx.get_i64(0)?);
    Ok(ctx.get_string(1)?.map(|s| s.len() as i64))
}

// ---------------------------------------------------------------------------
// SCALAR EMITS generators: gen_<class>_row / gen_<class>_batch (n, do_emit)
// ---------------------------------------------------------------------------

#[exasol_udf]
pub fn gen_native_row(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    let (n, do_emit) = scalar_params(ctx)?;
    generate_rows(ctx, n, do_emit, native_row)
}

#[exasol_udf]
pub fn gen_native_batch(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    let (n, do_emit) = scalar_params(ctx)?;
    generate_batches(ctx, n, do_emit, CHUNK, native_batch)
}

#[exasol_udf]
pub fn gen_strblock_row(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    let (n, do_emit) = scalar_params(ctx)?;
    generate_rows(ctx, n, do_emit, strblock_row)
}

#[exasol_udf]
pub fn gen_strblock_batch(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    let (n, do_emit) = scalar_params(ctx)?;
    generate_batches(ctx, n, do_emit, CHUNK, strblock_batch)
}

#[exasol_udf]
pub fn gen_varchar_row(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    let (n, do_emit) = scalar_params(ctx)?;
    generate_rows(ctx, n, do_emit, varchar_row)
}

#[exasol_udf]
pub fn gen_varchar_batch(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    let (n, do_emit) = scalar_params(ctx)?;
    generate_batches(ctx, n, do_emit, CHUNK, varchar_batch)
}

#[exasol_udf]
pub fn gen_wide_row(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    let (n, do_emit) = scalar_params(ctx)?;
    generate_rows(ctx, n, do_emit, wide_row)
}

/// `(n, do_emit, batch_rows)`: wide rows in `batch_rows`-row record batches.
#[exasol_udf]
pub fn gen_wide_batch(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    let (n, do_emit) = scalar_params(ctx)?;
    let chunk = batch_rows(ctx)?;
    generate_batches(ctx, n, do_emit, chunk, wide_batch)
}

// ---------------------------------------------------------------------------
// SCALAR EMITS pass-through: pt_native(k, n) EMITS (k_out DECIMAL(18,0))
// ---------------------------------------------------------------------------

/// Emits `n` rows carrying `k`. Selected beside `k`, every emitted row must
/// land next to the input row it came from.
#[exasol_udf]
pub fn pt_native(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    let k = ctx.get_i64(0)?.unwrap_or(0);
    let n = ctx.get_i64(1)?.unwrap_or(0);
    let row = [Value::Int64(k)];
    for _ in 0..n {
        ctx.emit(&row)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// SET RETURNS: set_sum_<class>
// ---------------------------------------------------------------------------

/// `(k, v) RETURNS DOUBLE`: sum of `v` over the group.
#[exasol_udf]
pub fn set_sum_native(ctx: &mut dyn UdfContext) -> Result<Option<f64>, UdfError> {
    let mut sum = 0.0f64;
    while ctx.next()? {
        std::hint::black_box(ctx.get_i64(0)?);
        sum += ctx.get_f64(1)?.unwrap_or(0.0);
    }
    Ok(Some(sum))
}

/// `(k, amount, d, ts) RETURNS DECIMAL(36,2)`: sum of `amount` over the group.
#[exasol_udf]
pub fn set_sum_strblock(ctx: &mut dyn UdfContext) -> Result<Option<Decimal>, UdfError> {
    let mut sum = 0i128;
    while ctx.next()? {
        std::hint::black_box(ctx.get_i64(0)?);
        sum += ctx.get_decimal(1)?.map(|d| d.unscaled).unwrap_or(0);
        std::hint::black_box(ctx.get_date(2)?);
        std::hint::black_box(ctx.get_timestamp(3)?);
    }
    Ok(Some(Decimal {
        unscaled: sum,
        scale: AMOUNT_SCALE,
    }))
}

// ---------------------------------------------------------------------------
// SET EMITS re-emitters: set_emit_<class>_row / _batch
// ---------------------------------------------------------------------------

#[exasol_udf]
pub fn set_emit_native_row(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    reemit_rows(ctx)
}

#[exasol_udf]
pub fn set_emit_native_batch(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    reemit_batches::<NativeCollector>(ctx)
}

#[exasol_udf]
pub fn set_emit_strblock_row(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    reemit_rows(ctx)
}

#[exasol_udf]
pub fn set_emit_strblock_batch(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    reemit_batches::<StrblockCollector>(ctx)
}

#[exasol_udf]
pub fn set_emit_varchar_row(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    reemit_rows(ctx)
}

#[exasol_udf]
pub fn set_emit_varchar_batch(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    reemit_batches::<VarcharCollector>(ctx)
}

// ---------------------------------------------------------------------------
// SET EMITS generators from one row: setgen_<class>_row / _batch (n, do_emit)
// ---------------------------------------------------------------------------

#[exasol_udf]
pub fn setgen_native_row(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    let (n, do_emit) = set_params(ctx)?;
    generate_rows(ctx, n, do_emit, native_row)
}

#[exasol_udf]
pub fn setgen_native_batch(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    let (n, do_emit) = set_params(ctx)?;
    generate_batches(ctx, n, do_emit, CHUNK, native_batch)
}

#[exasol_udf]
pub fn setgen_strblock_row(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    let (n, do_emit) = set_params(ctx)?;
    generate_rows(ctx, n, do_emit, strblock_row)
}

#[exasol_udf]
pub fn setgen_strblock_batch(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    let (n, do_emit) = set_params(ctx)?;
    generate_batches(ctx, n, do_emit, CHUNK, strblock_batch)
}

#[exasol_udf]
pub fn setgen_varchar_row(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    let (n, do_emit) = set_params(ctx)?;
    generate_rows(ctx, n, do_emit, varchar_row)
}

#[exasol_udf]
pub fn setgen_varchar_batch(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    let (n, do_emit) = set_params(ctx)?;
    generate_batches(ctx, n, do_emit, CHUNK, varchar_batch)
}

#[exasol_udf]
pub fn setgen_wide_row(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    let (n, do_emit) = set_params(ctx)?;
    generate_rows(ctx, n, do_emit, wide_row)
}

/// `(n, do_emit, batch_rows)` from the group's first row.
#[exasol_udf]
pub fn setgen_wide_batch(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    if !ctx.next()? {
        return Err(UdfError::User("bench-udfs: empty input group".into()));
    }
    let (n, do_emit) = scalar_params(ctx)?;
    let chunk = batch_rows(ctx)?;
    while ctx.next()? {}
    generate_batches(ctx, n, do_emit, chunk, wide_batch)
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
