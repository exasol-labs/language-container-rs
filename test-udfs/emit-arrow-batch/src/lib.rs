use std::sync::Arc;

use arrow::array::{
    ArrayRef, BooleanArray, Date32Array, Decimal128Array, Float64Array, Int32Array, Int64Array,
    LargeStringArray, StringArray, TimestampMicrosecondArray, TimestampMillisecondArray,
    TimestampNanosecondArray, TimestampSecondArray,
};
use arrow::datatypes::{DataType, Field, Schema, TimeUnit};
use arrow::record_batch::RecordBatch;
use chrono::NaiveDate;
use exasol_udf_macros::exasol_udf;
use exasol_udf_sdk::context::{EmitBatch, UdfContext};
use exasol_udf_sdk::error::UdfError;
use exasol_udf_sdk::value::{Decimal, Value};

/// SET UDF: drains all input rows then emits a fixed 3-row RecordBatch.
///
/// Emitted rows (in order):
///   id=1, label="a"
///   id=2, label="b"
///   id=3, label="c"
///
/// EMITS schema: `id BIGINT, label VARCHAR(1)`
#[exasol_udf]
pub fn emit_arrow_batch(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    while ctx.next()? {}

    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("label", DataType::Utf8, false),
    ]));
    let ids = Arc::new(Int64Array::from(vec![1i64, 2, 3]));
    let labels = Arc::new(StringArray::from(vec!["a", "b", "c"]));
    let batch = RecordBatch::try_new(schema, vec![ids, labels])
        .map_err(|e| UdfError::User(e.to_string()))?;

    ctx.emit_batch(&batch)
}

#[derive(Clone, Copy)]
enum Kind {
    I32,
    I64,
    F64,
    Bool,
    Utf8,
    LargeUtf8,
    Date32,
    Ts(TimeUnit),
    Dec(i8),
}

/// One column per path through the emit encoders. The SQL types live in the
/// integration test; `Kind` is the Arrow type `emit_types_batch` feeds it.
const COLUMNS: [(&str, Kind); 24] = [
    ("k", Kind::I32),
    ("c_i32", Kind::I32),
    ("c_i32w", Kind::I32),
    ("c_i64", Kind::I64),
    ("c_i64w", Kind::I64),
    ("c_dbl", Kind::F64),
    ("c_bool", Kind::Bool),
    ("c_big", Kind::I64),
    ("c_num_i32", Kind::I32),
    ("c_num_f64", Kind::F64),
    ("c_num_str", Kind::Utf8),
    ("c_dec0", Kind::Dec(0)),
    ("c_dec10", Kind::Dec(10)),
    ("c_dt", Kind::Date32),
    ("c_dt_str", Kind::Utf8),
    ("c_ts_s", Kind::Ts(TimeUnit::Second)),
    ("c_ts_ms", Kind::Ts(TimeUnit::Millisecond)),
    ("c_ts_us", Kind::Ts(TimeUnit::Microsecond)),
    ("c_ts_ns", Kind::Ts(TimeUnit::Nanosecond)),
    ("c_ts_str", Kind::Utf8),
    ("c_vc", Kind::Utf8),
    ("c_vc_large", Kind::LargeUtf8),
    ("c_vc_huge", Kind::Utf8),
    ("c_ch", Kind::Utf8),
];

const NINES_36: i128 = 999_999_999_999_999_999_999_999_999_999_999_999;

fn dec(unscaled: i128, scale: u8) -> Value {
    Value::Numeric(Decimal { unscaled, scale })
}

fn date(y: i32, m: u32, d: u32) -> Value {
    Value::Date(NaiveDate::from_ymd_opt(y, m, d).unwrap())
}

/// Without TIMESTAMP(9) support the fractions are cut to milliseconds.
fn ts(day: (i32, u32, u32), hms: (u32, u32, u32), nanos: u32, ns: bool) -> Value {
    let nanos = if ns { nanos } else { nanos - nanos % 1_000_000 };
    let d = NaiveDate::from_ymd_opt(day.0, day.1, day.2).unwrap();
    Value::Timestamp(d.and_hms_nano_opt(hms.0, hms.1, hms.2, nanos).unwrap())
}

fn s(v: &str) -> Value {
    Value::String(v.into())
}

fn type_rows(ns: bool) -> Vec<Vec<Value>> {
    use Value::{Bool, Double, Int32, Int64, Null};
    let leap = (2024, 2, 29);
    let noon = (12, 34, 56);
    let first = (1, 1, 1);
    let midnight = (0, 0, 0);
    let last = (9999, 12, 31);
    let eve = (1969, 12, 31);
    let end = (23, 59, 59);
    let mut rows = vec![
        vec![
            Int32(1),
            Int32(42),
            Int64(7),
            Int64(1_234_567_890_123),
            Int32(i32::MAX),
            Double(1.5),
            Bool(true),
            Int64(i64::MAX),
            Int32(i32::MAX),
            dec(123_456_789_012_345, 1),
            dec(1_234_500, 4),
            dec(42, 0),
            dec(31_415_926_535, 10),
            date(2024, 2, 29),
            date(2024, 2, 29),
            ts(leap, noon, 0, ns),
            ts(leap, noon, 123_000_000, ns),
            ts(leap, noon, 123_456_000, ns),
            ts(leap, noon, 123_456_789, ns),
            ts(leap, noon, 123_456_789, ns),
            s("a"),
            s("ñ日本語🦀"),
            Null,
            s("abc"),
        ],
        vec![
            Int32(2),
            Int32(-999_999_999),
            Int64(-999_999_999),
            Int64(-999_999_999_999_999_999),
            Int32(i32::MIN),
            Double(-2.25),
            Bool(false),
            Int64(i64::MIN),
            Int32(i32::MIN),
            dec(-625, 4),
            dec(-1, 4),
            dec(-NINES_36, 0),
            dec(-NINES_36, 10),
            date(1, 1, 1),
            date(1, 1, 1),
            ts(first, midnight, 0, ns),
            ts(first, midnight, 1_000_000, ns),
            ts(first, midnight, 1_000, ns),
            ts((1677, 9, 22), midnight, 1, ns),
            ts(first, midnight, 1, ns),
            s(&("日".repeat(42) + "x")),
            s(&"x".repeat(128)),
            Null,
            s("0123456789"),
        ],
        vec![
            Int32(3),
            Int32(999_999_999),
            Int64(0),
            Int64(999_999_999_999_999_999),
            Int32(0),
            Double(0.1),
            Bool(true),
            Int64(0),
            Int32(0),
            dec(0, 1),
            dec(0, 4),
            dec(NINES_36, 0),
            dec(-1, 10),
            date(9999, 12, 31),
            date(9999, 12, 31),
            ts(last, end, 0, ns),
            ts(last, end, 999_000_000, ns),
            ts(last, end, 999_999_000, ns),
            ts((2262, 4, 11), (23, 47, 16), 854_775_807, ns),
            ts(last, end, 999_999_999, ns),
            s(""),
            s(""),
            Null,
            s(""),
        ],
        vec![
            Int32(4),
            Null,
            Int64(1),
            Null,
            Int32(-1),
            Null,
            Bool(false),
            Null,
            Int32(-1),
            Null,
            dec(-1_234_500, 4),
            Null,
            dec(5_000_000_000, 10),
            Null,
            date(1969, 12, 31),
            Null,
            ts(eve, end, 999_000_000, ns),
            Null,
            ts(eve, end, 999_999_999, ns),
            Null,
            s(&"x".repeat(16_383)),
            s(&"é".repeat(8_192)),
            s(&"é".repeat(2_000_000)),
            Null,
        ],
    ];
    let mut nulls = vec![Null; COLUMNS.len()];
    nulls[0] = Int32(5);
    rows.push(nulls);
    rows
}

fn decimal_text(d: &Decimal) -> String {
    let digits = d.unscaled.unsigned_abs().to_string();
    let scale = d.scale as usize;
    let sign = if d.unscaled < 0 { "-" } else { "" };
    if scale == 0 {
        return format!("{sign}{digits}");
    }
    let padded = format!("{digits:0>width$}", width = scale + 1);
    let (int, frac) = padded.split_at(padded.len() - scale);
    format!("{sign}{int}.{frac}")
}

fn text(v: &Value) -> Option<String> {
    match v {
        Value::Null => None,
        Value::String(s) => Some(s.clone()),
        Value::Numeric(d) => Some(decimal_text(d)),
        Value::Date(d) => Some(d.format("%Y-%m-%d").to_string()),
        Value::Timestamp(t) => Some(t.format("%Y-%m-%d %H:%M:%S%.9f").to_string()),
        other => panic!("no text form for {other:?}"),
    }
}

fn int(v: &Value) -> Option<i64> {
    match v {
        Value::Int32(i) => Some(*i as i64),
        Value::Int64(i) => Some(*i),
        _ => None,
    }
}

fn column(kind: Kind, cells: Vec<&Value>) -> (DataType, ArrayRef) {
    let cells = cells.into_iter();
    match kind {
        Kind::I32 => (
            DataType::Int32,
            Arc::new(Int32Array::from_iter(
                cells.map(|v| int(v).map(|i| i as i32)),
            )),
        ),
        Kind::I64 => (
            DataType::Int64,
            Arc::new(Int64Array::from_iter(cells.map(int))),
        ),
        Kind::F64 => (
            DataType::Float64,
            Arc::new(Float64Array::from_iter(cells.map(|v| match v {
                Value::Double(f) => Some(*f),
                Value::Numeric(d) => Some(d.unscaled as f64 / 10f64.powi(d.scale as i32)),
                _ => None,
            }))),
        ),
        Kind::Bool => (
            DataType::Boolean,
            Arc::new(BooleanArray::from_iter(cells.map(|v| match v {
                Value::Bool(b) => Some(*b),
                _ => None,
            }))),
        ),
        Kind::Utf8 => (
            DataType::Utf8,
            Arc::new(StringArray::from_iter(cells.map(text))),
        ),
        Kind::LargeUtf8 => (
            DataType::LargeUtf8,
            Arc::new(LargeStringArray::from_iter(cells.map(text))),
        ),
        Kind::Date32 => {
            let epoch = NaiveDate::from_ymd_opt(1970, 1, 1).unwrap();
            (
                DataType::Date32,
                Arc::new(Date32Array::from_iter(cells.map(|v| match v {
                    Value::Date(d) => Some((*d - epoch).num_days() as i32),
                    _ => None,
                }))),
            )
        }
        Kind::Ts(unit) => {
            let values = cells.map(|v| match v {
                Value::Timestamp(t) => {
                    let secs = t.and_utc().timestamp();
                    let nanos = t.and_utc().timestamp_subsec_nanos() as i64;
                    Some(match unit {
                        TimeUnit::Second => secs,
                        TimeUnit::Millisecond => secs * 1_000 + nanos / 1_000_000,
                        TimeUnit::Microsecond => secs * 1_000_000 + nanos / 1_000,
                        TimeUnit::Nanosecond => secs * 1_000_000_000 + nanos,
                    })
                }
                _ => None,
            });
            let array: ArrayRef = match unit {
                TimeUnit::Second => Arc::new(TimestampSecondArray::from_iter(values)),
                TimeUnit::Millisecond => Arc::new(TimestampMillisecondArray::from_iter(values)),
                TimeUnit::Microsecond => Arc::new(TimestampMicrosecondArray::from_iter(values)),
                TimeUnit::Nanosecond => Arc::new(TimestampNanosecondArray::from_iter(values)),
            };
            (DataType::Timestamp(unit, None), array)
        }
        Kind::Dec(scale) => {
            let array = Decimal128Array::from_iter(cells.map(|v| match v {
                Value::Numeric(d) => Some(d.unscaled * 10i128.pow(scale as u32 - d.scale as u32)),
                _ => None,
            }))
            .with_precision_and_scale(36, scale)
            .unwrap();
            (DataType::Decimal128(36, scale), Arc::new(array))
        }
    }
}

fn type_batch(rows: &[Vec<Value>]) -> Result<RecordBatch, UdfError> {
    let (fields, arrays): (Vec<Field>, Vec<ArrayRef>) = COLUMNS
        .iter()
        .enumerate()
        .map(|(c, (name, kind))| {
            let (dt, array) = column(*kind, rows.iter().map(|r| &r[c]).collect());
            (Field::new(*name, dt, true), array)
        })
        .unzip();
    RecordBatch::try_new(Arc::new(Schema::new(fields)), arrays)
        .map_err(|e| UdfError::User(e.to_string()))
}

/// SCALAR EMITS: emits `type_rows` through `ctx.emit`. The argument says whether
/// the TIMESTAMP columns carry nanoseconds.
#[exasol_udf]
pub fn emit_types_row(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    let ns = ctx.get_bool(0)?.unwrap_or(false);
    for row in type_rows(ns) {
        ctx.emit(row)?;
    }
    Ok(())
}

/// SCALAR EMITS: `emit_types_row`'s rows through `ctx.emit_batch`, in two batches.
#[exasol_udf]
pub fn emit_types_batch(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    let ns = ctx.get_bool(0)?.unwrap_or(false);
    let rows = type_rows(ns);
    let (head, tail) = rows.split_at(2);
    ctx.emit_batch(&type_batch(head)?)?;
    ctx.emit_batch(&type_batch(tail)?)
}
