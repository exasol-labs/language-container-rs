//! Emit-throughput benchmark UDFs (Rust side).
//!
//! Two SET/EMITS entry points, both taking `(n BIGINT, do_emit BIGINT)` and
//! emitting the mixed shape `id BIGINT, label VARCHAR(100), val DOUBLE`:
//!
//! - `emit_mixed_row`   — row-at-a-time `ctx.emit`.
//! - `emit_mixed_batch` — columnar `ctx.emit_batch` (Arrow), ~100k rows/batch.
//!
//! The `do_emit` flag is what lets the driver isolate the three measure points:
//! with `do_emit = 0` the UDF builds all N rows (same per-row construction cost)
//! but emits a single sentinel instead of transferring the data, yielding the
//! generation cost; `do_emit = 1` generates *and* emits, so
//! `T_transfer = T_full − T_generation`.

use std::sync::Arc;

use arrow::array::{Float64Array, Int64Array, StringArray};
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use exasol_udf_macros::exasol_udf;
use exasol_udf_sdk::context::{EmitBatch, UdfContext};
use exasol_udf_sdk::error::UdfError;
use exasol_udf_sdk::value::Value;

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
