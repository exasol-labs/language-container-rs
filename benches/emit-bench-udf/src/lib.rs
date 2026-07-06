//! Emit-throughput benchmark UDFs (Rust side).
//!
//! Two shapes, each with a row-at-a-time and a columnar (`ctx.emit_batch`)
//! entry point, both taking `(n BIGINT, do_emit BIGINT)`:
//!
//! - **mixed** — `id BIGINT, label VARCHAR(100), val DOUBLE`
//!   (`emit_mixed_row` / `emit_mixed_batch`): the original shape, no
//!   NUMERIC/DATE/TIMESTAMP string-block columns.
//! - **wide** — `id BIGINT, amount DECIMAL(18,2), event_date DATE,
//!   event_ts TIMESTAMP, label VARCHAR(100)` (`emit_wide_row` /
//!   `emit_wide_batch`): exercises `value_to_block_string`'s chrono- and
//!   `Decimal`-`Display`-based formatting for all three string-block
//!   temporal/numeric types.
//!
//! The `do_emit` flag is what lets the driver isolate the three measure points:
//! with `do_emit = 0` the UDF builds all N rows (same per-row construction cost)
//! but emits a single sentinel instead of transferring the data, yielding the
//! generation cost; `do_emit = 1` generates *and* emits, so
//! `T_transfer = T_full − T_generation`.
//!
//! `sink_mixed` / `sink_wide` are the ingest-side counterpart: a SET script
//! that reads every column of every input row (forcing
//! `InputRowSet::from_proto` / `decode_string_block` to materialise it) and
//! emits a single row with the count. The driver chains
//! `sink_<shape>(emit_<shape>_<mode>(n, 1))` and subtracts the already-measured
//! emit `T_full` to isolate the ingest-only cost.

use std::sync::Arc;

use arrow::array::{
    Date32Array, Decimal128Array, Float64Array, Int64Array, StringArray, TimestampNanosecondArray,
};
use arrow::datatypes::{DataType, Field, Schema, TimeUnit};
use arrow::record_batch::RecordBatch;
use exasol_udf_macros::exasol_udf;
use exasol_udf_sdk::context::{EmitBatch, UdfContext};
use exasol_udf_sdk::error::UdfError;
use exasol_udf_sdk::value::{Decimal, Value};

/// 50-char payload, comfortably inside VARCHAR(100). Matches the "mixed" shape.
const LABEL: &str = "0123456789012345678901234567890123456789012345678";

/// Rows per Arrow batch in the columnar path. The host re-splits at the
/// 4,000,000-byte MT_EMIT limit anyway; this just bounds peak UDF-side memory.
const CHUNK: i64 = 100_000;

/// Read `(n, do_emit)` from the first input row, then drain the rest.
fn read_params(ctx: &mut dyn UdfContext) -> Result<(i64, bool), UdfError> {
    if !ctx.next()? {
        return Err(UdfError::User("emit-bench: no input row".into()));
    }
    let n = as_i64(ctx.get(0)?)?;
    let do_emit = as_i64(ctx.get(1)?)? != 0;
    while ctx.next()? {} // drain remaining input rows
    Ok((n, do_emit))
}

/// Accept BIGINT (Int64) or a scale-0 DECIMAL, like the other test UDFs.
fn as_i64(v: &Value) -> Result<i64, UdfError> {
    match v {
        Value::Int64(n) => Ok(*n),
        Value::Numeric(d) if d.scale == 0 => {
            i64::try_from(d.unscaled).map_err(|_| UdfError::Type("param overflow".into()))
        }
        other => Err(UdfError::Type(format!("expected BIGINT, got {other:?}"))),
    }
}

/// Build one mixed row's values. Pulled out so the emit and generate-only paths
/// pay the identical per-row construction cost (incl. the String allocation).
#[inline]
fn mixed_row(i: i64) -> [Value; 3] {
    [
        Value::Int64(i),
        Value::String(LABEL.to_string()),
        Value::Double(i as f64 * 1.5),
    ]
}

#[exasol_udf]
pub fn emit_mixed_row(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    let (n, do_emit) = read_params(ctx)?;
    if do_emit {
        for i in 0..n {
            ctx.emit(&mixed_row(i))?;
        }
    } else {
        // Generate every row (construction cost) but transfer nothing; black_box
        // keeps the optimizer from eliding the work. Emit one sentinel so the
        // query still returns.
        for i in 0..n {
            std::hint::black_box(mixed_row(i));
        }
        ctx.emit(&mixed_row(0))?;
    }
    Ok(())
}

#[exasol_udf]
pub fn emit_mixed_batch(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    let (n, do_emit) = read_params(ctx)?;
    let mut emitted = 0i64;
    while emitted < n {
        let len = (n - emitted).min(CHUNK);
        let batch = build_batch(emitted, len)?;
        if do_emit {
            ctx.emit_batch(&batch)?;
        } else {
            std::hint::black_box(&batch);
        }
        emitted += len;
    }
    if !do_emit {
        // Sentinel so the generate-only run still returns a row.
        ctx.emit_batch(&build_batch(0, 1)?)?;
    }
    Ok(())
}

/// Build a `len`-row mixed RecordBatch starting at id `start`.
fn build_batch(start: i64, len: i64) -> Result<RecordBatch, UdfError> {
    let len = len as usize;
    let ids: Vec<i64> = (0..len as i64).map(|k| start + k).collect();
    let vals: Vec<f64> = ids.iter().map(|&i| i as f64 * 1.5).collect();
    let labels: Vec<&str> = vec![LABEL; len];

    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("label", DataType::Utf8, false),
        Field::new("val", DataType::Float64, false),
    ]));
    RecordBatch::try_new(
        schema,
        vec![
            Arc::new(Int64Array::from(ids)),
            Arc::new(StringArray::from(labels)),
            Arc::new(Float64Array::from(vals)),
        ],
    )
    .map_err(|e| UdfError::User(e.to_string()))
}

/// Epoch (1970-01-01), the reference point for the wide shape's date/timestamp
/// arithmetic.
fn epoch_date() -> chrono::NaiveDate {
    chrono::NaiveDate::from_ymd_opt(1970, 1, 1).unwrap()
}

/// Base date/timestamp for the wide shape's generated values (2020-01-01).
fn wide_base_date() -> chrono::NaiveDate {
    chrono::NaiveDate::from_ymd_opt(2020, 1, 1).unwrap()
}
fn wide_base_ts() -> chrono::NaiveDateTime {
    wide_base_date().and_hms_opt(0, 0, 0).unwrap()
}

/// Row `i`'s `event_date`: cycles through a ~10-year window so
/// `value_to_block_string` sees varying `%Y-%m-%d` output, not one constant.
fn wide_event_date(i: i64) -> chrono::NaiveDate {
    wide_base_date() + chrono::Duration::days(i % 3650)
}

/// Row `i`'s `event_ts`: seconds advance with `i`, nanoseconds cycle through a
/// full range so the fractional-second formatting path is exercised too.
fn wide_event_ts(i: i64) -> chrono::NaiveDateTime {
    let nanos = (i as u32).wrapping_mul(137) % 1_000_000_000;
    wide_base_ts() + chrono::Duration::seconds(i) + chrono::Duration::nanoseconds(nanos as i64)
}

/// Row `i`'s `amount`: a DECIMAL(18,2) value that grows with `i`.
fn wide_amount(i: i64) -> Decimal {
    Decimal {
        unscaled: (i as i128) * 137 + 4200,
        scale: 2,
    }
}

fn date_to_epoch_days(d: chrono::NaiveDate) -> i32 {
    d.signed_duration_since(epoch_date()).num_days() as i32
}

fn ts_to_epoch_nanos(ts: chrono::NaiveDateTime) -> i64 {
    ts.and_utc().timestamp_nanos_opt().unwrap()
}

/// Build one wide row's values: `id, amount, event_date, event_ts, label`.
#[inline]
fn wide_row(i: i64) -> [Value; 5] {
    [
        Value::Int64(i),
        Value::Numeric(wide_amount(i)),
        Value::Date(wide_event_date(i)),
        Value::Timestamp(wide_event_ts(i)),
        Value::String(LABEL.to_string()),
    ]
}

#[exasol_udf]
pub fn emit_wide_row(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    let (n, do_emit) = read_params(ctx)?;
    if do_emit {
        for i in 0..n {
            ctx.emit(&wide_row(i))?;
        }
    } else {
        for i in 0..n {
            std::hint::black_box(wide_row(i));
        }
        ctx.emit(&wide_row(0))?;
    }
    Ok(())
}

#[exasol_udf]
pub fn emit_wide_batch(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    let (n, do_emit) = read_params(ctx)?;
    let mut emitted = 0i64;
    while emitted < n {
        let len = (n - emitted).min(CHUNK);
        let batch = build_wide_batch(emitted, len)?;
        if do_emit {
            ctx.emit_batch(&batch)?;
        } else {
            std::hint::black_box(&batch);
        }
        emitted += len;
    }
    if !do_emit {
        ctx.emit_batch(&build_wide_batch(0, 1)?)?;
    }
    Ok(())
}

/// Build a `len`-row wide RecordBatch starting at id `start`.
fn build_wide_batch(start: i64, len: i64) -> Result<RecordBatch, UdfError> {
    let len = len as usize;
    let ids: Vec<i64> = (0..len as i64).map(|k| start + k).collect();
    let amounts: Vec<i128> = ids.iter().map(|&i| wide_amount(i).unscaled).collect();
    let dates: Vec<i32> = ids
        .iter()
        .map(|&i| date_to_epoch_days(wide_event_date(i)))
        .collect();
    let timestamps: Vec<i64> = ids
        .iter()
        .map(|&i| ts_to_epoch_nanos(wide_event_ts(i)))
        .collect();
    let labels: Vec<&str> = vec![LABEL; len];

    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("amount", DataType::Decimal128(18, 2), false),
        Field::new("event_date", DataType::Date32, false),
        Field::new(
            "event_ts",
            DataType::Timestamp(TimeUnit::Nanosecond, None),
            false,
        ),
        Field::new("label", DataType::Utf8, false),
    ]));
    let amount_array = Decimal128Array::from(amounts)
        .with_precision_and_scale(18, 2)
        .map_err(|e| UdfError::User(e.to_string()))?;
    RecordBatch::try_new(
        schema,
        vec![
            Arc::new(Int64Array::from(ids)),
            Arc::new(amount_array),
            Arc::new(Date32Array::from(dates)),
            Arc::new(TimestampNanosecondArray::from(timestamps)),
            Arc::new(StringArray::from(labels)),
        ],
    )
    .map_err(|e| UdfError::User(e.to_string()))
}

/// Ingest-side counterpart of `emit_mixed_*`: read every column of every
/// input row (forcing full string-block decode) and emit the row count.
#[exasol_udf]
pub fn sink_mixed(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    let mut count: i64 = 0;
    while ctx.next()? {
        std::hint::black_box(ctx.get(0)?);
        std::hint::black_box(ctx.get(1)?);
        std::hint::black_box(ctx.get(2)?);
        count += 1;
    }
    ctx.emit(&[Value::Int64(count)])?;
    Ok(())
}

/// Ingest-side counterpart of `emit_wide_*`: read every column of every
/// input row (forcing full NUMERIC/DATE/TIMESTAMP string-block decode) and
/// emit the row count.
#[exasol_udf]
pub fn sink_wide(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    let mut count: i64 = 0;
    while ctx.next()? {
        for col in 0..5 {
            std::hint::black_box(ctx.get(col)?);
        }
        count += 1;
    }
    ctx.emit(&[Value::Int64(count)])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wide_row_has_five_typed_columns_with_expected_variants() {
        let row = wide_row(42);
        assert!(matches!(row[0], Value::Int64(42)));
        assert!(matches!(row[1], Value::Numeric(_)));
        assert!(matches!(row[2], Value::Date(_)));
        assert!(matches!(row[3], Value::Timestamp(_)));
        assert!(matches!(row[4], Value::String(_)));
    }

    #[test]
    fn wide_amount_keeps_scale_two() {
        assert_eq!(wide_amount(0).scale, 2);
        assert_eq!(wide_amount(1_000_000).scale, 2);
    }

    #[test]
    fn wide_event_date_cycles_within_ten_years() {
        let d0 = wide_event_date(0);
        let d1 = wide_event_date(3650);
        assert_eq!(d0, d1, "date arithmetic must wrap every 3650 days");
    }

    #[test]
    fn date_epoch_conversion_round_trips() {
        let d = wide_event_date(123);
        let days = date_to_epoch_days(d);
        assert_eq!(epoch_date() + chrono::Duration::days(days as i64), d);
    }

    #[test]
    fn timestamp_epoch_conversion_is_monotonic_in_seconds() {
        let nanos_0 = ts_to_epoch_nanos(wide_event_ts(0));
        let nanos_1 = ts_to_epoch_nanos(wide_event_ts(1));
        assert!(
            nanos_1 > nanos_0,
            "advancing i by one second must advance the epoch-nanos timestamp"
        );
        assert!(
            nanos_1 - nanos_0 >= 999_000_000,
            "one second of wall-clock advance must show up as ~1e9 ns, got {}",
            nanos_1 - nanos_0
        );
    }

    #[test]
    fn build_wide_batch_produces_expected_shape() {
        let batch = build_wide_batch(0, 5).unwrap();
        assert_eq!(batch.num_rows(), 5);
        assert_eq!(batch.num_columns(), 5);
    }

    #[test]
    fn build_wide_batch_dates_match_row_path() {
        let batch = build_wide_batch(10, 3).unwrap();
        let dates = batch
            .column(2)
            .as_any()
            .downcast_ref::<Date32Array>()
            .unwrap();
        for k in 0..3 {
            assert_eq!(
                dates.value(k),
                date_to_epoch_days(wide_event_date(10 + k as i64))
            );
        }
    }
}
