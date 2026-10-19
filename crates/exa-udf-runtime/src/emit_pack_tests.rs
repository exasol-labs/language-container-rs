use std::sync::Arc;

use arrow::array::{
    ArrayRef, BooleanArray, Date32Array, Decimal128Array, Float64Array, Int32Array, Int64Array,
    LargeStringArray, StringArray, TimestampMicrosecondArray, TimestampMillisecondArray,
    TimestampNanosecondArray, TimestampSecondArray,
};
use arrow::datatypes::{DataType, Field, Schema, TimeUnit};
use chrono::{Datelike, NaiveDate};
use exa_zmq_protocol::{EmitRequest, ExaType, Frame};
use exasol_udf_sdk::value::Value;

use super::*;
use crate::rowset::EmitBuffer;

fn col(name: &str, typ: ExaType) -> ColumnInfo {
    ColumnInfo {
        name: name.to_string(),
        typ,
        type_name: String::new(),
        size: None,
        precision: None,
        scale: None,
    }
}

fn numeric(scale: u32) -> ExaType {
    ExaType::Numeric {
        precision: 38,
        scale,
    }
}

fn timestamp_type() -> ExaType {
    ExaType::Timestamp { precision: 9 }
}

fn batch(fields: Vec<(&str, DataType, bool)>, arrays: Vec<ArrayRef>) -> RecordBatch {
    let schema = Arc::new(Schema::new(
        fields
            .into_iter()
            .map(|(n, t, nullable)| Field::new(n, t, nullable))
            .collect::<Vec<_>>(),
    ));
    RecordBatch::try_new(schema, arrays).unwrap()
}

/// The `Value` row the row path would carry for `r`, read through
/// `accessor_value` (the reference decoding). A valid cell of an
/// `Unsupported` column is any non-NULL value: neither path gives it a slot.
fn reference_row(b: &RecordBatch, meta: &[ColumnInfo], r: usize) -> Vec<Value> {
    let accessors = build_accessors(b, meta).unwrap();
    accessors
        .iter()
        .enumerate()
        .map(|(c, acc)| match acc {
            _ if b.column(c).is_null(r) => Value::Null,
            ColAccessor::Unsupported => Value::Bool(false),
            _ => accessor_value(acc, r),
        })
        .collect()
}

/// Every `MT_EMIT` frame the buffer flushed, then the frame of what it still
/// holds.
fn frames_via_batch(b: &RecordBatch, meta: &[ColumnInfo], row_number: u64) -> Vec<Vec<u8>> {
    let mut frames = Vec::new();
    let mut buf = EmitBuffer::new();
    buf.push_batch(b, meta, row_number, &mut |t| {
        frames.push(frame(t));
        Ok(())
    })
    .unwrap();
    buf.flush(|t| {
        frames.push(frame(t));
        Ok::<(), UdfError>(())
    })
    .unwrap();
    frames
}

fn frames_via_rows(b: &RecordBatch, meta: &[ColumnInfo], row_number: u64) -> Vec<Vec<u8>> {
    let mut frames = Vec::new();
    let mut buf = EmitBuffer::new();
    for r in 0..b.num_rows() {
        buf.push(reference_row(b, meta, r), row_number, meta);
        if buf.should_flush() {
            buf.flush(|t| {
                frames.push(frame(t));
                Ok::<(), UdfError>(())
            })
            .unwrap();
        }
    }
    buf.flush(|t| {
        frames.push(frame(t));
        Ok::<(), UdfError>(())
    })
    .unwrap();
    frames
}

fn frame(t: &EmitTable) -> Vec<u8> {
    EmitRequest {
        connection_id: 9,
        table: t,
    }
    .encode_frame()
}

fn assert_batch_matches_rows(b: &RecordBatch, meta: &[ColumnInfo]) {
    let via_batch = frames_via_batch(b, meta, 3);
    let via_rows = frames_via_rows(b, meta, 3);
    assert_eq!(via_batch.len(), via_rows.len(), "flush count");
    for (i, (a, e)) in via_batch.iter().zip(&via_rows).enumerate() {
        assert!(
            a == e,
            "frame {i} differs ({} vs {} bytes)",
            a.len(),
            e.len()
        );
    }
}

fn days_since_epoch(d: NaiveDate) -> i32 {
    (d - NaiveDate::from_ymd_opt(1970, 1, 1).unwrap()).num_days() as i32
}

fn written(f: impl FnOnce(&mut Vec<u8>) -> bool) -> Option<String> {
    let mut out = Vec::new();
    f(&mut out).then(|| String::from_utf8(out).unwrap())
}

#[test]
fn year_cache_matches_chrono_for_every_day_of_years_0_to_9999_in_order() {
    let mut years = YearCache::default();
    let first = days_since_epoch(NaiveDate::from_ymd_opt(0, 1, 1).unwrap());
    let last = days_since_epoch(NaiveDate::from_ymd_opt(9999, 12, 31).unwrap());
    for days in first..=last {
        let expected =
            NaiveDate::from_num_days_from_ce_opt(days + crate::rowset::ARROW_EPOCH_CE_DAY).unwrap();
        assert_eq!(
            years.civil(i64::from(days)),
            (i64::from(expected.year()), expected.month(), expected.day()),
            "day {days}"
        );
    }
}

#[test]
fn date32_writer_matches_chrono_format_on_edge_dates() {
    for date in [
        NaiveDate::from_ymd_opt(0, 1, 1).unwrap(),
        NaiveDate::from_ymd_opt(1, 1, 1).unwrap(),
        NaiveDate::from_ymd_opt(1969, 12, 31).unwrap(),
        NaiveDate::from_ymd_opt(1970, 1, 1).unwrap(),
        NaiveDate::from_ymd_opt(2000, 2, 29).unwrap(),
        NaiveDate::from_ymd_opt(2024, 2, 29).unwrap(),
        NaiveDate::from_ymd_opt(9999, 12, 31).unwrap(),
    ] {
        let fast =
            written(|out| write_date32(out, days_since_epoch(date), &mut YearCache::default()))
                .unwrap();
        assert_eq!(fast, date.format(crate::rowset::DATE_FORMAT).to_string());
    }
}

#[test]
fn date32_writer_defers_outside_years_0_to_9999() {
    let before = days_since_epoch(NaiveDate::from_ymd_opt(0, 1, 1).unwrap()) - 1;
    let after = days_since_epoch(NaiveDate::from_ymd_opt(9999, 12, 31).unwrap()) + 1;
    for days in [before, after, i32::MIN, i32::MAX] {
        let mut out = b"keep".to_vec();
        assert!(
            !write_date32(&mut out, days, &mut YearCache::default()),
            "day {days}"
        );
        assert_eq!(out, b"keep", "day {days} must write nothing");
    }
}

#[test]
fn timestamp_writer_matches_chrono_format_on_edge_instants() {
    let mut years = YearCache::default();
    let cases: [(i64, u32); 9] = [
        (0, 0),
        (-1, 999_999_999),
        (-62_167_219_200, 0),           // 0000-01-01 00:00:00
        (253_402_300_799, 999_999_999), // 9999-12-31 23:59:59.999999999
        (1_709_164_800, 4),             // 2024-02-29 00:00:00.000000004
        (-86_401, 1),
        (951_782_400, 123_456_789),
        (86_399, 0),
        (1_700_000_000, 500_000_000),
    ];
    for (secs, nanos) in cases {
        let expected = chrono::DateTime::from_timestamp(secs, nanos)
            .unwrap()
            .naive_utc()
            .format(crate::rowset::TIMESTAMP_EMIT)
            .to_string();
        let fast = written(|out| write_timestamp(out, secs, nanos, &mut years)).unwrap();
        assert_eq!(fast, expected, "secs {secs} nanos {nanos}");
    }
}

#[test]
fn timestamp_writer_defers_outside_years_0_to_9999() {
    for secs in [-62_167_219_201, 253_402_300_800, i64::MIN, i64::MAX] {
        let mut out = b"keep".to_vec();
        assert!(
            !write_timestamp(&mut out, secs, 0, &mut YearCache::default()),
            "secs {secs}"
        );
        assert_eq!(out, b"keep", "secs {secs} must write nothing");
    }
}

#[test]
fn timestamp_parts_split_every_unit_like_chrono() {
    let b = batch(
        vec![
            ("s", DataType::Timestamp(TimeUnit::Second, None), false),
            (
                "ms",
                DataType::Timestamp(TimeUnit::Millisecond, None),
                false,
            ),
            (
                "us",
                DataType::Timestamp(TimeUnit::Microsecond, None),
                false,
            ),
            ("ns", DataType::Timestamp(TimeUnit::Nanosecond, None), false),
        ],
        vec![
            Arc::new(TimestampSecondArray::from(vec![-1i64, 7])),
            Arc::new(TimestampMillisecondArray::from(vec![-1i64, 1_001])),
            Arc::new(TimestampMicrosecondArray::from(vec![-1i64, 1_000_001])),
            Arc::new(TimestampNanosecondArray::from(vec![-1i64, 1_000_000_001])),
        ],
    );
    let meta: Vec<ColumnInfo> = ["s", "ms", "us", "ns"]
        .iter()
        .map(|n| col(n, timestamp_type()))
        .collect();
    let packer = BatchPacker::new(&b, &meta).unwrap();
    let expected = [
        [(-1, 0), (7, 0)],
        [(-1, 999_000_000), (1, 1_000_000)],
        [(-1, 999_999_000), (1, 1_000)],
        [(-1, 999_999_999), (1, 1)],
    ];
    for (c, sc) in packer.strings.iter().enumerate() {
        for (r, want) in expected[c].iter().enumerate() {
            assert_eq!(timestamp_parts(&sc.kind, r), *want, "column {c} row {r}");
            let Value::Timestamp(reference) = accessor_value(&packer.accessors[c], r) else {
                panic!("timestamp expected");
            };
            let (secs, nanos) = timestamp_parts(&sc.kind, r);
            assert_eq!(
                chrono::DateTime::from_timestamp(secs, nanos)
                    .unwrap()
                    .naive_utc(),
                reference,
                "column {c} row {r}"
            );
        }
    }
}

/// Every `ColAccessor` variant, every timestamp unit, negative and pre-epoch
/// values, scale 0 and 9-digit fractions, 2024-02-29, NULLs and empty
/// strings, with two columns sharing each block.
fn every_variant_batch() -> (RecordBatch, Vec<ColumnInfo>) {
    let d = |y, m, dd| days_since_epoch(NaiveDate::from_ymd_opt(y, m, dd).unwrap());
    let fields = vec![
        ("i32", DataType::Int32, true),
        ("i64", DataType::Int64, true),
        ("f64", DataType::Float64, true),
        ("b", DataType::Boolean, true),
        ("s", DataType::Utf8, true),
        ("ls", DataType::LargeUtf8, true),
        ("d", DataType::Date32, true),
        ("ts_s", DataType::Timestamp(TimeUnit::Second, None), true),
        (
            "ts_ms",
            DataType::Timestamp(TimeUnit::Millisecond, None),
            true,
        ),
        (
            "ts_us",
            DataType::Timestamp(TimeUnit::Microsecond, None),
            true,
        ),
        (
            "ts_ns",
            DataType::Timestamp(TimeUnit::Nanosecond, None),
            true,
        ),
        ("dec0", DataType::Decimal128(38, 0), true),
        ("dec10", DataType::Decimal128(38, 10), true),
        ("n_i32", DataType::Int32, true),
        ("n_i64", DataType::Int64, true),
        ("n_f64", DataType::Float64, true),
        ("i64b", DataType::Int64, false),
        ("bb", DataType::Boolean, false),
        ("unsupported", DataType::Int64, true),
    ];
    let meta = vec![
        col("i32", ExaType::Int32),
        col("i64", ExaType::Int64),
        col("f64", ExaType::Double),
        col("b", ExaType::Boolean),
        col("s", ExaType::String { size: 100 }),
        col("ls", ExaType::Char { size: 100 }),
        col("d", ExaType::Date),
        col("ts_s", timestamp_type()),
        col("ts_ms", timestamp_type()),
        col("ts_us", timestamp_type()),
        col("ts_ns", timestamp_type()),
        col("dec0", numeric(0)),
        col("dec10", numeric(10)),
        col("n_i32", numeric(0)),
        col("n_i64", numeric(0)),
        col("n_f64", numeric(0)),
        col("i64b", ExaType::Int64),
        col("bb", ExaType::Boolean),
        col("unsupported", ExaType::Unsupported),
    ];
    let arrays: Vec<ArrayRef> = vec![
        Arc::new(Int32Array::from(vec![
            Some(-7),
            None,
            Some(i32::MAX),
            Some(0),
        ])),
        Arc::new(Int64Array::from(vec![
            None,
            Some(i64::MIN),
            Some(42),
            Some(-1),
        ])),
        Arc::new(Float64Array::from(vec![
            Some(-0.5),
            Some(1e300),
            None,
            Some(0.0),
        ])),
        Arc::new(BooleanArray::from(vec![
            Some(true),
            None,
            Some(false),
            Some(true),
        ])),
        Arc::new(StringArray::from(vec![
            Some(""),
            Some("héllo"),
            None,
            Some("x"),
        ])),
        Arc::new(LargeStringArray::from(vec![
            Some("large"),
            None,
            Some(""),
            Some("y"),
        ])),
        Arc::new(Date32Array::from(vec![
            Some(d(2024, 2, 29)),
            Some(d(1969, 7, 20)),
            None,
            Some(d(0, 1, 1)),
        ])),
        Arc::new(TimestampSecondArray::from(vec![
            Some(-1),
            None,
            Some(253_402_300_799),
            Some(0),
        ])),
        Arc::new(TimestampMillisecondArray::from(vec![
            Some(-1),
            Some(1_709_164_800_001),
            None,
            Some(999),
        ])),
        Arc::new(TimestampMicrosecondArray::from(vec![
            None,
            Some(-1_000_001),
            Some(1),
            Some(951_782_400_123_456),
        ])),
        Arc::new(TimestampNanosecondArray::from(vec![
            Some(-1),
            Some(1_709_164_800_000_000_004),
            Some(i64::MIN),
            None,
        ])),
        Arc::new(
            Decimal128Array::from(vec![Some(-1i128), Some(0), None, Some(i128::MAX)])
                .with_precision_and_scale(38, 0)
                .unwrap(),
        ),
        Arc::new(
            Decimal128Array::from(vec![
                Some(-1i128),
                Some(12_345_678_901_234_567_890_123_456_789),
                Some(1),
                None,
            ])
            .with_precision_and_scale(38, 10)
            .unwrap(),
        ),
        Arc::new(Int32Array::from(vec![Some(-3), Some(3), None, Some(0)])),
        Arc::new(Int64Array::from(vec![
            Some(i64::MAX),
            None,
            Some(-4),
            Some(0),
        ])),
        Arc::new(Float64Array::from(vec![
            None,
            Some(2.5),
            Some(-1e-7),
            Some(0.0),
        ])),
        Arc::new(Int64Array::from(vec![1i64, 2, 3, 4])),
        Arc::new(BooleanArray::from(vec![false, false, true, true])),
        Arc::new(Int64Array::from(vec![Some(1i64), None, Some(3), Some(4)])),
    ];
    (batch(fields, arrays), meta)
}

#[test]
fn pack_matches_row_path_for_every_accessor_variant() {
    let (b, meta) = every_variant_batch();
    assert_batch_matches_rows(&b, &meta);
}

#[test]
fn pack_matches_row_path_for_a_row_range() {
    let (b, meta) = every_variant_batch();
    let packer = BatchPacker::new(&b, &meta).unwrap();
    let mut via_packer = EmitTable::default();
    packer.pack(1..3, 5, &mut via_packer);

    let mut via_rows = EmitBuffer::new();
    for r in 1..3 {
        via_rows.push(reference_row(&b, &meta, r), 5, &meta);
    }
    let mut expected = EmitTable::default();
    via_rows
        .flush(|t| {
            expected = t.clone();
            Ok::<(), UdfError>(())
        })
        .unwrap();
    assert_eq!(via_packer, expected);
}

#[test]
fn pack_of_an_empty_range_appends_nothing() {
    let (b, meta) = every_variant_batch();
    let packer = BatchPacker::new(&b, &meta).unwrap();
    let mut out = EmitTable::default();
    packer.pack(2..2, 5, &mut out);
    assert_eq!(out, EmitTable::default());
}

struct Lcg(u64);

impl Lcg {
    fn next(&mut self, bound: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) % bound
    }
}

/// Random batches over every block type, with NULLs, empty strings and one
/// row carrying a string over the 4,000,000-byte flush target, must flush at
/// the same rows as the row path and produce the same frames.
#[test]
fn pack_matches_row_path_for_random_batches_with_an_oversized_row() {
    let mut rng = Lcg(7);
    for round in 0..4 {
        let n = 200 + rng.next(300) as usize;
        let oversized_row = rng.next(n as u64) as usize;
        let mut i64s = Vec::new();
        let mut f64s = Vec::new();
        let mut bools = Vec::new();
        let mut strs: Vec<Option<String>> = Vec::new();
        let mut dates = Vec::new();
        let mut tss = Vec::new();
        let mut decs = Vec::new();
        let mut i64s2 = Vec::new();
        for r in 0..n {
            let null = |rng: &mut Lcg| rng.next(5) == 0;
            i64s.push((!null(&mut rng)).then(|| rng.next(u64::MAX) as i64));
            f64s.push((!null(&mut rng)).then(|| rng.next(1_000_000) as f64 / 7.0 - 500.0));
            bools.push((!null(&mut rng)).then(|| rng.next(2) == 1));
            strs.push(if r == oversized_row {
                Some("z".repeat(4_000_001))
            } else if null(&mut rng) {
                None
            } else {
                let len = rng.next(300) as usize;
                Some(
                    (0..len)
                        .map(|_| b'a' + rng.next(26) as u8)
                        .map(char::from)
                        .collect(),
                )
            });
            dates.push((!null(&mut rng)).then(|| rng.next(4_000_000) as i32 - 1_000_000));
            tss.push((!null(&mut rng)).then(|| rng.next(u64::MAX) as i64 / 4));
            decs.push((!null(&mut rng)).then(|| {
                let v = rng.next(u64::MAX) as i128 * if rng.next(2) == 0 { 1 } else { -1 };
                if rng.next(4) == 0 { v * v } else { v }
            }));
            i64s2.push((!null(&mut rng)).then(|| rng.next(1_000) as i64));
        }
        let b = batch(
            vec![
                ("a", DataType::Int64, true),
                ("f", DataType::Float64, true),
                ("b", DataType::Boolean, true),
                ("s", DataType::Utf8, true),
                ("d", DataType::Date32, true),
                ("ts", DataType::Timestamp(TimeUnit::Nanosecond, None), true),
                ("dec", DataType::Decimal128(38, 3), true),
                ("a2", DataType::Int64, true),
            ],
            vec![
                Arc::new(Int64Array::from(i64s)),
                Arc::new(Float64Array::from(f64s)),
                Arc::new(BooleanArray::from(bools)),
                Arc::new(StringArray::from(strs)),
                Arc::new(Date32Array::from(dates)),
                Arc::new(TimestampNanosecondArray::from(tss)),
                Arc::new(
                    Decimal128Array::from(decs)
                        .with_precision_and_scale(38, 3)
                        .unwrap(),
                ),
                Arc::new(Int64Array::from(i64s2)),
            ],
        );
        let meta = vec![
            col("a", ExaType::Int64),
            col("f", ExaType::Double),
            col("b", ExaType::Boolean),
            col("s", ExaType::String { size: 5_000_000 }),
            col("d", ExaType::Date),
            col("ts", timestamp_type()),
            col("dec", numeric(3)),
            col("a2", ExaType::Int64),
        ];
        let frames = frames_via_batch(&b, &meta, 1);
        assert!(
            frames.len() >= 2,
            "round {round}: the oversized row must force a flush"
        );
        assert_batch_matches_rows(&b, &meta);
    }
}

#[test]
fn pack_fixed_single_null_free_column_is_a_slice_copy() {
    let b = batch(
        vec![("a", DataType::Int64, false)],
        vec![Arc::new(Int64Array::from(vec![5i64, 6, 7, 8]))],
    );
    let meta = vec![col("a", ExaType::Int64)];
    let packer = BatchPacker::new(&b, &meta).unwrap();
    let mut out = EmitTable::default();
    packer.pack(1..4, 0, &mut out);
    assert_eq!(out.int64, vec![6, 7, 8]);
    assert_eq!(out.nulls, vec![false; 3]);
    assert_eq!(out.rows, 3);
    assert_eq!(out.row_numbers, vec![0; 3]);
}

#[test]
fn string_bytes_hint_covers_the_packed_string_block() {
    let (b, meta) = every_variant_batch();
    let packer = BatchPacker::new(&b, &meta).unwrap();
    let hint = packer.string_bytes(&(0..4));
    let mut out = EmitTable::default();
    packer.pack(0..4, 0, &mut out);
    let packed = frame(&out).len();
    assert!(
        hint >= packed / 2,
        "hint {hint} far below the packed size {packed}"
    );
}

#[test]
fn date32_writer_matches_chrono_format_across_years_0_to_9999() {
    let mut years = YearCache::default();
    let first = days_since_epoch(NaiveDate::from_ymd_opt(0, 1, 1).unwrap());
    let last = days_since_epoch(NaiveDate::from_ymd_opt(9999, 12, 31).unwrap());
    for days in (first..=last).step_by(37) {
        let expected =
            NaiveDate::from_num_days_from_ce_opt(days + crate::rowset::ARROW_EPOCH_CE_DAY)
                .unwrap()
                .format(crate::rowset::DATE_FORMAT)
                .to_string();
        let fast = written(|out| write_date32(out, days, &mut years)).unwrap();
        assert_eq!(fast, expected, "day {days}");
    }
}

#[test]
fn timestamp_writer_matches_chrono_format_across_years_0_to_9999() {
    let mut years = YearCache::default();
    let mut rng = Lcg(11);
    let first = -62_167_219_200i64;
    let last = 253_402_300_799i64;
    for secs in (first..=last).step_by(2_654_321) {
        let nanos = rng.next(1_000_000_000) as u32;
        let expected = chrono::DateTime::from_timestamp(secs, nanos)
            .unwrap()
            .naive_utc()
            .format(crate::rowset::TIMESTAMP_EMIT)
            .to_string();
        let fast = written(|out| write_timestamp(out, secs, nanos, &mut years)).unwrap();
        assert_eq!(fast, expected, "secs {secs} nanos {nanos}");
    }
}

#[test]
fn year_cache_matches_chrono_for_days_in_random_order() {
    let mut years = YearCache::default();
    let mut rng = Lcg(3);
    for _ in 0..200_000 {
        let days = rng.next(4_000_000) as i64 - 800_000;
        let expected =
            NaiveDate::from_num_days_from_ce_opt(days as i32 + crate::rowset::ARROW_EPOCH_CE_DAY)
                .unwrap();
        assert_eq!(
            years.civil(days),
            (i64::from(expected.year()), expected.month(), expected.day()),
            "day {days}"
        );
        assert_eq!(civil_from_days(days), years.civil(days), "day {days}");
    }
}
