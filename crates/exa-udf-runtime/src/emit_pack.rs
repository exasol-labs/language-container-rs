//! Packs an Arrow `RecordBatch` into the wire's type blocks a column at a time,
//! byte-identical to the row path (`EmitBuffer::push`).

use std::ops::Range;

use arrow::array::Array;
use arrow::buffer::{BooleanBuffer, NullBuffer};
use arrow::record_batch::RecordBatch;
use exa_zmq_protocol::{ColumnInfo, EmitTable};
use exasol_udf_sdk::error::UdfError;
use exasol_udf_sdk::value::Decimal;

use crate::rowset::{
    BYTES_DATE, BYTES_TIMESTAMP, ColAccessor, NUMERIC_COST_BASE, accessor_value, batch_byte_cost,
    build_accessors, compute_row_costs, write_block_value, write_decimal, write_hms_nanos,
    write_ymd,
};

struct FixedCol<'a, T> {
    values: &'a [T],
    nulls: Option<&'a NullBuffer>,
}

struct BoolCol<'a> {
    values: &'a BooleanBuffer,
    nulls: Option<&'a NullBuffer>,
}

/// Arrow buffers resolved once per batch, so the cell loop never re-dispatches.
struct StringCol<'a> {
    col: usize,
    nulls: Option<&'a NullBuffer>,
    kind: StringKind<'a>,
}

enum StringKind<'a> {
    Utf8 {
        offsets: &'a [i32],
        data: &'a [u8],
    },
    LargeUtf8 {
        offsets: &'a [i64],
        data: &'a [u8],
    },
    Date32(&'a [i32]),
    TsSecond(&'a [i64]),
    TsMillisecond(&'a [i64]),
    TsMicrosecond(&'a [i64]),
    TsNanosecond(&'a [i64]),
    Decimal {
        values: &'a [i128],
        scale: u8,
    },
    /// Stringified via `Value`, like the row path.
    Widened,
}

pub(crate) struct BatchPacker<'a> {
    batch: &'a RecordBatch,
    meta: &'a [ColumnInfo],
    accessors: Vec<ColAccessor<'a>>,
    nulls: Vec<Option<&'a NullBuffer>>,
    int32: Vec<FixedCol<'a, i32>>,
    int64: Vec<FixedCol<'a, i64>>,
    doubles: Vec<FixedCol<'a, f64>>,
    bools: Vec<BoolCol<'a>>,
    strings: Vec<StringCol<'a>>,
}

impl<'a> BatchPacker<'a> {
    /// Fails before any row lands in a block.
    pub(crate) fn new(batch: &'a RecordBatch, meta: &'a [ColumnInfo]) -> Result<Self, UdfError> {
        let accessors = build_accessors(batch, meta)?;
        let nulls: Vec<Option<&'a NullBuffer>> = (0..meta.len())
            .map(|c| batch.column(c).nulls().filter(|nb| nb.null_count() > 0))
            .collect();
        let mut packer = BatchPacker {
            batch,
            meta,
            accessors,
            nulls,
            int32: Vec::new(),
            int64: Vec::new(),
            doubles: Vec::new(),
            bools: Vec::new(),
            strings: Vec::new(),
        };
        for (c, acc) in packer.accessors.iter().enumerate() {
            let nulls = packer.nulls[c];
            match acc {
                ColAccessor::Int32(arr) => packer.int32.push(FixedCol {
                    values: arr.values(),
                    nulls,
                }),
                ColAccessor::Int64(arr) => packer.int64.push(FixedCol {
                    values: arr.values(),
                    nulls,
                }),
                ColAccessor::Float64(arr) => packer.doubles.push(FixedCol {
                    values: arr.values(),
                    nulls,
                }),
                ColAccessor::Boolean(arr) => packer.bools.push(BoolCol {
                    values: arr.values(),
                    nulls,
                }),
                ColAccessor::Unsupported => {}
                string => {
                    let kind = match string {
                        ColAccessor::Utf8(arr) => StringKind::Utf8 {
                            offsets: arr.value_offsets(),
                            data: arr.value_data(),
                        },
                        ColAccessor::LargeUtf8(arr) => StringKind::LargeUtf8 {
                            offsets: arr.value_offsets(),
                            data: arr.value_data(),
                        },
                        ColAccessor::Date32(arr) => StringKind::Date32(arr.values()),
                        ColAccessor::TsSecond(arr) => StringKind::TsSecond(arr.values()),
                        ColAccessor::TsMillisecond(arr) => StringKind::TsMillisecond(arr.values()),
                        ColAccessor::TsMicrosecond(arr) => StringKind::TsMicrosecond(arr.values()),
                        ColAccessor::TsNanosecond(arr) => StringKind::TsNanosecond(arr.values()),
                        ColAccessor::Decimal128(arr, scale) => StringKind::Decimal {
                            values: arr.values(),
                            scale: *scale as u8,
                        },
                        ColAccessor::NumericFromInt32(_)
                        | ColAccessor::NumericFromInt64(_)
                        | ColAccessor::NumericFromFloat64(_) => StringKind::Widened,
                        ColAccessor::Int32(_)
                        | ColAccessor::Int64(_)
                        | ColAccessor::Float64(_)
                        | ColAccessor::Boolean(_)
                        | ColAccessor::Unsupported => unreachable!("fixed-width column"),
                    };
                    packer.strings.push(StringCol {
                        col: c,
                        nulls,
                        kind,
                    });
                }
            }
        }
        Ok(packer)
    }

    /// An upper bound on the sum of `row_costs`.
    pub(crate) fn byte_cost(&self) -> usize {
        batch_byte_cost(self.batch, self.meta)
    }

    pub(crate) fn row_costs(&self) -> Vec<usize> {
        compute_row_costs(self.batch, self.meta)
    }

    pub(crate) fn pack(&self, rows: Range<usize>, row_number: u64, out: &mut EmitTable) {
        let n = rows.len();
        if n == 0 {
            return;
        }
        self.pack_nulls(rows.clone(), out);
        pack_fixed(&self.int32, rows.clone(), &mut out.int32);
        pack_fixed(&self.int64, rows.clone(), &mut out.int64);
        pack_fixed(&self.doubles, rows.clone(), &mut out.doubles);
        pack_bools(&self.bools, rows.clone(), &mut out.bools);
        self.pack_strings(rows, out);
        out.row_numbers.extend(std::iter::repeat_n(row_number, n));
        out.rows += n as u64;
    }

    fn pack_nulls(&self, rows: Range<usize>, out: &mut EmitTable) {
        let n_cols = self.accessors.len();
        let base = out.nulls.len();
        out.nulls.resize(base + rows.len() * n_cols, false);
        for (c, nb) in self.nulls.iter().enumerate() {
            let Some(nb) = nb else { continue };
            for (i, valid) in validity(nb, &rows).iter().enumerate() {
                if !valid {
                    out.nulls[base + i * n_cols + c] = true;
                }
            }
        }
    }

    fn pack_strings(&self, rows: Range<usize>, out: &mut EmitTable) {
        if self.strings.is_empty() {
            return;
        }
        out.reserve_strings(self.string_bytes(&rows));
        let mut years = YearCache::default();
        for r in rows {
            for sc in &self.strings {
                if sc.nulls.is_some_and(|nb| nb.is_null(r)) {
                    continue;
                }
                match sc.kind {
                    StringKind::Utf8 { offsets, data } => {
                        out.push_string(&data[offsets[r] as usize..offsets[r + 1] as usize]);
                    }
                    StringKind::LargeUtf8 { offsets, data } => {
                        out.push_string(&data[offsets[r] as usize..offsets[r + 1] as usize]);
                    }
                    StringKind::Date32(values) => {
                        let days = values[r];
                        out.push_string_with(|buf| {
                            if !write_date32(buf, days, &mut years) {
                                self.slow_cell(buf, sc.col, r);
                            }
                        });
                    }
                    StringKind::TsSecond(_)
                    | StringKind::TsMillisecond(_)
                    | StringKind::TsMicrosecond(_)
                    | StringKind::TsNanosecond(_) => {
                        let (secs, nanos) = timestamp_parts(&sc.kind, r);
                        out.push_string_with(|buf| {
                            if !write_timestamp(buf, secs, nanos, &mut years) {
                                self.slow_cell(buf, sc.col, r);
                            }
                        });
                    }
                    StringKind::Decimal { values, scale } => {
                        let d = Decimal {
                            unscaled: values[r],
                            scale,
                        };
                        out.push_string_with(|buf| write_decimal(buf, &d));
                    }
                    StringKind::Widened => {
                        out.push_string_with(|buf| self.slow_cell(buf, sc.col, r));
                    }
                }
            }
        }
    }

    /// The row path's rendering: what the fast writers must match and defer to.
    fn slow_cell(&self, buf: &mut Vec<u8>, col: usize, r: usize) {
        write_block_value(buf, &accessor_value(&self.accessors[col], r));
    }

    fn string_bytes(&self, rows: &Range<usize>) -> usize {
        let n = rows.len();
        self.strings
            .iter()
            .map(|sc| match sc.kind {
                StringKind::Utf8 { offsets, .. } => {
                    (offsets[rows.end] - offsets[rows.start]) as usize + 3 * n
                }
                StringKind::LargeUtf8 { offsets, .. } => {
                    (offsets[rows.end] - offsets[rows.start]) as usize + 3 * n
                }
                StringKind::Date32(_) => n * (BYTES_DATE + 2),
                StringKind::TsSecond(_)
                | StringKind::TsMillisecond(_)
                | StringKind::TsMicrosecond(_)
                | StringKind::TsNanosecond(_) => n * (BYTES_TIMESTAMP + 2),
                StringKind::Decimal { scale, .. } => n * (NUMERIC_COST_BASE + scale as usize + 2),
                StringKind::Widened => n * (NUMERIC_COST_BASE + 2),
            })
            .sum()
    }
}

fn validity(nb: &NullBuffer, rows: &Range<usize>) -> BooleanBuffer {
    nb.inner().slice(rows.start, rows.len())
}

fn pack_fixed<T: Copy>(cols: &[FixedCol<'_, T>], rows: Range<usize>, block: &mut Vec<T>) {
    match cols {
        [] => {}
        [col] => match col.nulls {
            None => block.extend_from_slice(&col.values[rows]),
            Some(nb) => {
                block.reserve(rows.len());
                let bits = validity(nb, &rows);
                for (v, valid) in col.values[rows].iter().zip(bits.iter()) {
                    if valid {
                        block.push(*v);
                    }
                }
            }
        },
        cols => {
            block.reserve(rows.len() * cols.len());
            for r in rows {
                for col in cols {
                    if col.nulls.is_none_or(|nb| nb.is_valid(r)) {
                        block.push(col.values[r]);
                    }
                }
            }
        }
    }
}

fn pack_bools(cols: &[BoolCol<'_>], rows: Range<usize>, block: &mut Vec<bool>) {
    match cols {
        [] => {}
        [col] => match col.nulls {
            None => block.extend(col.values.slice(rows.start, rows.len()).iter()),
            Some(nb) => {
                block.reserve(rows.len());
                let values = col.values.slice(rows.start, rows.len());
                for (v, valid) in values.iter().zip(validity(nb, &rows).iter()) {
                    if valid {
                        block.push(v);
                    }
                }
            }
        },
        cols => {
            block.reserve(rows.len() * cols.len());
            for r in rows {
                for col in cols {
                    if col.nulls.is_none_or(|nb| nb.is_valid(r)) {
                        block.push(col.values.value(r));
                    }
                }
            }
        }
    }
}

/// The same euclidean split `accessor_value` hands `chrono`.
fn timestamp_parts(kind: &StringKind<'_>, r: usize) -> (i64, u32) {
    match kind {
        StringKind::TsSecond(values) => (values[r], 0),
        StringKind::TsMillisecond(values) => {
            let v = values[r];
            (
                v.div_euclid(1_000),
                (v.rem_euclid(1_000) * 1_000_000) as u32,
            )
        }
        StringKind::TsMicrosecond(values) => {
            let v = values[r];
            (
                v.div_euclid(1_000_000),
                (v.rem_euclid(1_000_000) * 1_000) as u32,
            )
        }
        StringKind::TsNanosecond(values) => {
            let v = values[r];
            (
                v.div_euclid(1_000_000_000),
                v.rem_euclid(1_000_000_000) as u32,
            )
        }
        _ => unreachable!("timestamp_parts on a non-timestamp column"),
    }
}

/// Howard Hinnant's `civil_from_days`.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month as u32, day as u32)
}

fn is_leap(year: i64) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

const DAYS_BEFORE_MONTH: [[u16; 12]; 2] = [
    [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334],
    [0, 31, 60, 91, 121, 152, 182, 213, 244, 274, 305, 335],
];

const MONTH_OF_DOY: [[u8; 366]; 2] = {
    let mut table = [[0u8; 366]; 2];
    let mut leap = 0;
    while leap < 2 {
        let mut month = 0;
        while month < 12 {
            let start = DAYS_BEFORE_MONTH[leap][month] as usize;
            let end = if month < 11 {
                DAYS_BEFORE_MONTH[leap][month + 1] as usize
            } else {
                365 + leap
            };
            let mut doy = start;
            while doy < end {
                table[leap][doy] = month as u8 + 1;
                doy += 1;
            }
            month += 1;
        }
        leap += 1;
    }
    table
};

/// Emitted dates cluster, so caching the last year turns most conversions
/// into two table lookups.
#[derive(Default)]
struct YearCache {
    start: i64,
    /// Zero until the first conversion.
    len: i64,
    year: i64,
    leap: usize,
}

impl YearCache {
    fn civil(&mut self, days: i64) -> (i64, u32, u32) {
        let mut doy = days - self.start;
        if doy < 0 || doy >= self.len {
            let (year, month, day) = civil_from_days(days);
            let leap = usize::from(is_leap(year));
            doy = i64::from(DAYS_BEFORE_MONTH[leap][month as usize - 1]) + i64::from(day) - 1;
            *self = YearCache {
                start: days - doy,
                len: 365 + leap as i64,
                year,
                leap,
            };
        }
        let month = MONTH_OF_DOY[self.leap][doy as usize];
        let day = doy as u32 - u32::from(DAYS_BEFORE_MONTH[self.leap][month as usize - 1]) + 1;
        (self.year, u32::from(month), day)
    }
}

/// Returns `false`, writing nothing, outside years `0..=9999`; the caller
/// then takes the `chrono` path.
fn write_date32(out: &mut Vec<u8>, days: i32, years: &mut YearCache) -> bool {
    let (year, month, day) = years.civil(i64::from(days));
    if !(0..=9999).contains(&year) {
        return false;
    }
    write_ymd(out, year as u32, month, day);
    true
}

/// Same `false` contract as `write_date32`.
fn write_timestamp(out: &mut Vec<u8>, secs: i64, nanos: u32, years: &mut YearCache) -> bool {
    let days = secs.div_euclid(86_400);
    let (year, month, day) = years.civil(days);
    if !(0..=9999).contains(&year) {
        return false;
    }
    let sod = secs.rem_euclid(86_400) as u32;
    write_ymd(out, year as u32, month, day);
    out.push(b' ');
    write_hms_nanos(out, sod / 3_600, sod / 60 % 60, sod % 60, nanos);
    true
}

#[cfg(test)]
#[path = "emit_pack_tests.rs"]
mod tests;
