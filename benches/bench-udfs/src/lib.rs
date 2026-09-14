//! Benchmark UDF entry points shared by both tiers; see `benches/README.md`.
//! Generators take `(n, do_emit[, batch_rows])`; `do_emit = 0` builds every row
//! and emits one sentinel so transfer cost is total minus this.

use std::sync::Arc;

use arrow::array::{
    ArrayRef, BooleanArray, Date32Array, Decimal128Array, Float64Array, Int64Array, StringArray,
    TimestampNanosecondArray,
};
use arrow::datatypes::{DataType, Field, Schema, TimeUnit};
use arrow::record_batch::RecordBatch;
use chrono::{Duration, NaiveDate, NaiveDateTime};
use exasol_udf_macros::exasol_udf;
use exasol_udf_sdk::context::{EmitBatch, UdfContext};
use exasol_udf_sdk::error::UdfError;
use exasol_udf_sdk::value::{Decimal, Value};

pub use bench_schema::{WIDE_BATCH_ROWS, WIDE_COLUMNS, wide_ddl};

const LABEL_LEN: usize = 50;
/// Rows per record batch unless the script passes `batch_rows`; bounds UDF-side memory.
const CHUNK: usize = 100_000;
const NULL_EVERY: i64 = 10;
const AMOUNT_SCALE: i8 = 2;
const BIG_SCALE: i8 = 10;
const WIDE_TEXT_MAX: [usize; 8] = [8, 16, 32, 32, 64, 64, 128, 200];
const WIDE_TEXT: &str = "the quick brown fox jumps over the lazy dog while forty-two \
    benchmark rows stream through a script language container, each carrying a \
    reference to a file whose contents are emitted as wide rows with dozens of \
    columns of mixed types and a sprinkling of nulls";

// --- cell values ------------------------------------------------------------

fn native_v(i: i64) -> f64 {
    i as f64 * 1.5
}

fn amount(i: i64) -> Decimal {
    Decimal {
        unscaled: i as i128 * 137 + 4200,
        scale: AMOUNT_SCALE as u8,
    }
}

fn big(i: i64) -> Decimal {
    Decimal {
        unscaled: i as i128 * 12_345_678_901 + 987_654_321,
        scale: BIG_SCALE as u8,
    }
}

fn base() -> NaiveDateTime {
    NaiveDate::from_ymd_opt(2020, 1, 1)
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .expect("valid base")
}

fn date(i: i64) -> NaiveDate {
    base().date() + Duration::days(i % 3650)
}

fn timestamp(i: i64) -> NaiveDateTime {
    base() + Duration::seconds(i) + Duration::microseconds((i * 137).rem_euclid(1_000_000))
}

fn label(i: i64) -> String {
    format!("{i:0>LABEL_LEN$}")
}

fn native_row(i: i64) -> [Value; 2] {
    [Value::Int64(i), Value::Double(native_v(i))]
}

fn strblock_row(i: i64) -> [Value; 4] {
    [
        Value::Int64(i),
        Value::Numeric(amount(i)),
        Value::Date(date(i)),
        Value::Timestamp(timestamp(i)),
    ]
}

fn varchar_row(i: i64) -> [Value; 2] {
    [Value::Int64(i), Value::String(label(i))]
}

fn wide_is_null(i: i64, col: usize) -> bool {
    WIDE_COLUMNS[col].2 && (i + col as i64) % NULL_EVERY == 0
}

fn wide_text(i: i64, col: usize) -> String {
    let max = WIDE_TEXT_MAX[col - 16];
    let len = 1 + (i as usize * 31 + col * 7) % max;
    let start = (i as usize * 13 + col) % (WIDE_TEXT.len() - len);
    WIDE_TEXT[start..start + len].to_string()
}

fn wide_cell(i: i64, col: usize) -> Value {
    if wide_is_null(i, col) {
        return Value::Null;
    }
    match col {
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

fn wide_row(i: i64) -> Vec<Value> {
    (0..WIDE_COLUMNS.len()).map(|c| wide_cell(i, c)).collect()
}

// --- Arrow batches ----------------------------------------------------------

fn epoch_days(d: NaiveDate) -> i32 {
    (d - NaiveDate::from_ymd_opt(1970, 1, 1).expect("epoch")).num_days() as i32
}

fn epoch_nanos(ts: NaiveDateTime) -> i64 {
    ts.and_utc()
        .timestamp_nanos_opt()
        .expect("nanosecond range")
}

fn arrow_err(e: impl std::fmt::Display) -> UdfError {
    UdfError::User(e.to_string())
}

fn arrow_type(sql: &str) -> DataType {
    match sql {
        "DECIMAL(18,0)" => DataType::Int64,
        "DOUBLE" => DataType::Float64,
        "BOOLEAN" => DataType::Boolean,
        "DECIMAL(18,2)" => DataType::Decimal128(18, AMOUNT_SCALE),
        "DECIMAL(36,10)" => DataType::Decimal128(36, BIG_SCALE),
        "DATE" => DataType::Date32,
        "TIMESTAMP" => DataType::Timestamp(TimeUnit::Nanosecond, None),
        _ => DataType::Utf8,
    }
}

fn schema(cols: &[(&str, &str, bool)]) -> Arc<Schema> {
    Arc::new(Schema::new(
        cols.iter()
            .map(|(name, ty, nullable)| Field::new(*name, arrow_type(ty), *nullable))
            .collect::<Vec<_>>(),
    ))
}

fn native_schema() -> Arc<Schema> {
    schema(&[("k", "DECIMAL(18,0)", false), ("v", "DOUBLE", false)])
}

fn strblock_schema() -> Arc<Schema> {
    schema(&[
        ("k", "DECIMAL(18,0)", false),
        ("amount", "DECIMAL(18,2)", false),
        ("d", "DATE", false),
        ("ts", "TIMESTAMP", false),
    ])
}

fn varchar_schema() -> Arc<Schema> {
    schema(&[
        ("k", "DECIMAL(18,0)", false),
        ("label", "VARCHAR(100)", false),
    ])
}

fn decimal<V>(values: V, precision: u8, scale: i8) -> Result<ArrayRef, UdfError>
where
    Decimal128Array: From<V>,
{
    Ok(Arc::new(
        Decimal128Array::from(values)
            .with_precision_and_scale(precision, scale)
            .map_err(arrow_err)?,
    ))
}

fn native_batch(start: i64, len: usize) -> Result<RecordBatch, UdfError> {
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

fn strblock_batch(start: i64, len: usize) -> Result<RecordBatch, UdfError> {
    let ks: Vec<i64> = (start..start + len as i64).collect();
    let amounts: Vec<i128> = ks.iter().map(|&i| amount(i).unscaled).collect();
    let amounts = decimal(amounts, 18, AMOUNT_SCALE)?;
    let dates: Vec<i32> = ks.iter().map(|&i| epoch_days(date(i))).collect();
    let tss: Vec<i64> = ks.iter().map(|&i| epoch_nanos(timestamp(i))).collect();
    RecordBatch::try_new(
        strblock_schema(),
        vec![
            Arc::new(Int64Array::from(ks)),
            amounts,
            Arc::new(Date32Array::from(dates)),
            Arc::new(TimestampNanosecondArray::from(tss)),
        ],
    )
    .map_err(arrow_err)
}

fn varchar_batch(start: i64, len: usize) -> Result<RecordBatch, UdfError> {
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

fn array(sql: &str, values: impl Iterator<Item = Value>) -> Result<ArrayRef, UdfError> {
    macro_rules! opt {
        ($arr:ty, $pat:pat => $val:expr) => {
            Arc::new(<$arr>::from_iter(values.map(|v| match v {
                $pat => Some($val),
                _ => None,
            })))
        };
    }
    Ok(match sql {
        "DECIMAL(18,0)" => opt!(Int64Array, Value::Int64(x) => x),
        "DOUBLE" => opt!(Float64Array, Value::Double(x) => x),
        "BOOLEAN" => opt!(BooleanArray, Value::Bool(x) => x),
        "DECIMAL(18,2)" | "DECIMAL(36,10)" => {
            let unscaled: Vec<Option<i128>> = values
                .map(|v| match v {
                    Value::Numeric(d) => Some(d.unscaled),
                    _ => None,
                })
                .collect();
            match sql {
                "DECIMAL(18,2)" => decimal(unscaled, 18, AMOUNT_SCALE)?,
                _ => decimal(unscaled, 36, BIG_SCALE)?,
            }
        }
        "DATE" => opt!(Date32Array, Value::Date(d) => epoch_days(d)),
        "TIMESTAMP" => opt!(TimestampNanosecondArray, Value::Timestamp(t) => epoch_nanos(t)),
        _ => opt!(StringArray, Value::String(s) => s),
    })
}

fn wide_batch(start: i64, len: usize) -> Result<RecordBatch, UdfError> {
    let cols = WIDE_COLUMNS
        .iter()
        .enumerate()
        .map(|(col, (_, ty, _))| array(ty, (start..start + len as i64).map(|i| wide_cell(i, col))))
        .collect::<Result<Vec<_>, _>>()?;
    RecordBatch::try_new(schema(&WIDE_COLUMNS), cols).map_err(arrow_err)
}

// --- drivers ----------------------------------------------------------------

fn scalar_params(ctx: &dyn UdfContext) -> Result<(i64, bool, usize), UdfError> {
    let n = ctx
        .get_i64(0)?
        .ok_or_else(|| UdfError::User("bench-udfs: n is NULL".into()))?;
    let do_emit = ctx.get_i64(1)?.unwrap_or(0) != 0;
    let chunk = match ctx.num_columns() > 2 {
        true => ctx
            .get_i64(2)?
            .filter(|&b| b > 0)
            .map_or(CHUNK, |b| b as usize),
        false => CHUNK,
    };
    Ok((n, do_emit, chunk))
}

fn set_params(ctx: &mut dyn UdfContext) -> Result<(i64, bool, usize), UdfError> {
    if !ctx.next()? {
        return Err(UdfError::User("bench-udfs: empty input group".into()));
    }
    let params = scalar_params(ctx)?;
    while ctx.next()? {}
    Ok(params)
}

fn generate_rows<R: Into<Vec<Value>>>(
    ctx: &mut dyn UdfContext,
    n: i64,
    do_emit: bool,
    row: impl Fn(i64) -> R,
) -> Result<(), UdfError> {
    if do_emit {
        for i in 0..n {
            ctx.emit(row(i).into())?;
        }
    } else {
        for i in 0..n {
            std::hint::black_box(row(i));
        }
        ctx.emit(row(0).into())?;
    }
    Ok(())
}

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

fn reemit_rows(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    while ctx.next()? {
        let row: Vec<Value> = (0..ctx.num_columns())
            .map(|c| ctx.get(c).cloned())
            .collect::<Result<_, _>>()?;
        ctx.emit(row)?;
    }
    Ok(())
}

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
            .push(ctx.get_decimal(1)?.map_or(0, |d| d.unscaled));
        self.d.push(ctx.get_date(2)?.map_or(0, epoch_days));
        self.ts.push(ctx.get_timestamp(3)?.map_or(0, epoch_nanos));
        Ok(())
    }
    fn len(&self) -> usize {
        self.k.len()
    }
    fn take_batch(&mut self) -> Result<RecordBatch, UdfError> {
        RecordBatch::try_new(
            strblock_schema(),
            vec![
                Arc::new(Int64Array::from(std::mem::take(&mut self.k))),
                decimal(std::mem::take(&mut self.amount), 18, AMOUNT_SCALE)?,
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

fn reemit_batches<C: ColumnCollector>(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    let mut col = C::default();
    while ctx.next()? {
        col.push(ctx)?;
        if col.len() == CHUNK {
            ctx.emit_batch(&col.take_batch()?)?;
        }
    }
    if col.len() > 0 {
        ctx.emit_batch(&col.take_batch()?)?;
    }
    Ok(())
}

// --- entry points -----------------------------------------------------------

#[exasol_udf]
pub fn sr_native(ctx: &mut dyn UdfContext) -> Result<Option<i64>, UdfError> {
    let k = ctx.get_i64(0)?;
    std::hint::black_box(ctx.get_f64(1)?);
    Ok(k.map(|k| k + 1))
}

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

#[exasol_udf]
pub fn sr_varchar(ctx: &mut dyn UdfContext) -> Result<Option<i64>, UdfError> {
    std::hint::black_box(ctx.get_i64(0)?);
    Ok(ctx.get_string(1)?.map(|s| s.len() as i64))
}

macro_rules! generators {
    ($($scalar:ident, $set:ident => rows $row:expr;)*) => {$(
        #[exasol_udf]
        pub fn $scalar(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
            let (n, do_emit, _) = scalar_params(ctx)?;
            generate_rows(ctx, n, do_emit, $row)
        }
        #[exasol_udf]
        pub fn $set(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
            let (n, do_emit, _) = set_params(ctx)?;
            generate_rows(ctx, n, do_emit, $row)
        }
    )*};
    ($($scalar:ident, $set:ident => batches $batch:expr;)*) => {$(
        #[exasol_udf]
        pub fn $scalar(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
            let (n, do_emit, chunk) = scalar_params(ctx)?;
            generate_batches(ctx, n, do_emit, chunk, $batch)
        }
        #[exasol_udf]
        pub fn $set(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
            let (n, do_emit, chunk) = set_params(ctx)?;
            generate_batches(ctx, n, do_emit, chunk, $batch)
        }
    )*};
}

generators! {
    gen_native_row, setgen_native_row => rows native_row;
    gen_strblock_row, setgen_strblock_row => rows strblock_row;
    gen_varchar_row, setgen_varchar_row => rows varchar_row;
    gen_wide_row, setgen_wide_row => rows wide_row;
}

generators! {
    gen_native_batch, setgen_native_batch => batches native_batch;
    gen_strblock_batch, setgen_strblock_batch => batches strblock_batch;
    gen_varchar_batch, setgen_varchar_batch => batches varchar_batch;
    gen_wide_batch, setgen_wide_batch => batches wide_batch;
}

/// `pt_native(k, n) EMITS (k_out)`: `n` copies of `k`, which must land beside their input row.
#[exasol_udf]
pub fn pt_native(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    let k = ctx.get_i64(0)?.unwrap_or(0);
    let n = ctx.get_i64(1)?.unwrap_or(0);
    for _ in 0..n {
        ctx.emit(vec![Value::Int64(k)])?;
    }
    Ok(())
}

#[exasol_udf]
pub fn set_sum_native(ctx: &mut dyn UdfContext) -> Result<Option<f64>, UdfError> {
    let mut sum = 0.0f64;
    while ctx.next()? {
        std::hint::black_box(ctx.get_i64(0)?);
        sum += ctx.get_f64(1)?.unwrap_or(0.0);
    }
    Ok(Some(sum))
}

#[exasol_udf]
pub fn set_sum_strblock(ctx: &mut dyn UdfContext) -> Result<Option<Decimal>, UdfError> {
    let mut sum = 0i128;
    while ctx.next()? {
        std::hint::black_box(ctx.get_i64(0)?);
        sum += ctx.get_decimal(1)?.map_or(0, |d| d.unscaled);
        std::hint::black_box(ctx.get_date(2)?);
        std::hint::black_box(ctx.get_timestamp(3)?);
    }
    Ok(Some(Decimal {
        unscaled: sum,
        scale: AMOUNT_SCALE as u8,
    }))
}

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

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
