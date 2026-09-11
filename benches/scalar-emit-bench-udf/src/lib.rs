//! SCALAR EMITS benchmark UDFs.
//!
//! Three shapes, each taking `(n BIGINT, do_emit BIGINT)` via a SCALAR script
//! called from DUAL (one invocation emits N rows):
//!
//! - **mixed** — `id BIGINT, label VARCHAR(100), val DOUBLE`
//! - **wide** — `id BIGINT, amount DECIMAL(18,2), event_date DATE,
//!   event_ts TIMESTAMP, label VARCHAR(100)`
//! - **native** — `id DECIMAL(18,0), val DOUBLE`
//!
//! `do_emit=1` generates and emits all N rows; `do_emit=0` generates all N rows
//! but emits a single sentinel, isolating the generation cost so the driver can
//! compute `T_transfer = T_full − T_generation`.

use exasol_udf_macros::exasol_udf;
use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::error::UdfError;
use exasol_udf_sdk::value::{Decimal, Value};

const LABEL: &str = "01234567890123456789012345678901234567890123456789";

fn wide_base_date() -> chrono::NaiveDate {
    chrono::NaiveDate::from_ymd_opt(2020, 1, 1).unwrap()
}
fn wide_base_ts() -> chrono::NaiveDateTime {
    wide_base_date().and_hms_opt(0, 0, 0).unwrap()
}
fn wide_event_date(i: i64) -> chrono::NaiveDate {
    wide_base_date() + chrono::Duration::days(i % 3650)
}
fn wide_event_ts(i: i64) -> chrono::NaiveDateTime {
    let nanos = (i as u32).wrapping_mul(137) % 1_000_000_000;
    wide_base_ts() + chrono::Duration::seconds(i) + chrono::Duration::nanoseconds(nanos as i64)
}
fn wide_amount(i: i64) -> Decimal {
    Decimal {
        unscaled: (i as i128) * 137 + 4200,
        scale: 2,
    }
}

#[exasol_udf]
pub fn scalar_emit_mixed(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    let n = ctx.get_i64(0)?.unwrap_or(0);
    let do_emit = ctx.get_i64(1)?.unwrap_or(0) != 0;
    if do_emit {
        for i in 0..n {
            ctx.emit(vec![
                Value::Int64(i),
                Value::String(LABEL.to_string()),
                Value::Double(i as f64 * 1.5),
            ])?;
        }
    } else {
        for i in 0..n {
            std::hint::black_box(vec![
                Value::Int64(i),
                Value::String(LABEL.to_string()),
                Value::Double(i as f64 * 1.5),
            ]);
        }
        ctx.emit(vec![
            Value::Int64(0),
            Value::String(LABEL.to_string()),
            Value::Double(0.0),
        ])?;
    }
    Ok(())
}

#[exasol_udf]
pub fn scalar_emit_wide(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    let n = ctx.get_i64(0)?.unwrap_or(0);
    let do_emit = ctx.get_i64(1)?.unwrap_or(0) != 0;
    if do_emit {
        for i in 0..n {
            ctx.emit(vec![
                Value::Int64(i),
                Value::Numeric(wide_amount(i)),
                Value::Date(wide_event_date(i)),
                Value::Timestamp(wide_event_ts(i)),
                Value::String(LABEL.to_string()),
            ])?;
        }
    } else {
        for i in 0..n {
            std::hint::black_box(vec![
                Value::Int64(i),
                Value::Numeric(wide_amount(i)),
                Value::Date(wide_event_date(i)),
                Value::Timestamp(wide_event_ts(i)),
                Value::String(LABEL.to_string()),
            ]);
        }
        ctx.emit(vec![
            Value::Int64(0),
            Value::Numeric(wide_amount(0)),
            Value::Date(wide_event_date(0)),
            Value::Timestamp(wide_event_ts(0)),
            Value::String(LABEL.to_string()),
        ])?;
    }
    Ok(())
}

#[exasol_udf]
pub fn scalar_emit_native(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    let n = ctx.get_i64(0)?.unwrap_or(0);
    let do_emit = ctx.get_i64(1)?.unwrap_or(0) != 0;
    if do_emit {
        for i in 0..n {
            ctx.emit(vec![Value::Int64(i), Value::Double(i as f64 * 1.5)])?;
        }
    } else {
        for i in 0..n {
            std::hint::black_box(vec![Value::Int64(i), Value::Double(i as f64 * 1.5)]);
        }
        ctx.emit(vec![Value::Int64(0), Value::Double(0.0)])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use exasol_udf_sdk::test_support::TestContext;

    #[test]
    fn mixed_emits_n_rows() {
        let mut ctx = TestContext::scalar(vec![Value::Int64(3), Value::Int64(1)]);
        scalar_emit_mixed(&mut ctx).unwrap();
        assert_eq!(ctx.emitted().len(), 3);
    }

    #[test]
    fn mixed_sentinel_emits_one_row() {
        let mut ctx = TestContext::scalar(vec![Value::Int64(100), Value::Int64(0)]);
        scalar_emit_mixed(&mut ctx).unwrap();
        assert_eq!(ctx.emitted().len(), 1);
    }

    #[test]
    fn wide_emits_n_rows() {
        let mut ctx = TestContext::scalar(vec![Value::Int64(3), Value::Int64(1)]);
        scalar_emit_wide(&mut ctx).unwrap();
        assert_eq!(ctx.emitted().len(), 3);
        assert!(matches!(ctx.emitted()[0][1], Value::Numeric(_)));
        assert!(matches!(ctx.emitted()[0][2], Value::Date(_)));
        assert!(matches!(ctx.emitted()[0][3], Value::Timestamp(_)));
    }

    #[test]
    fn native_emits_n_rows() {
        let mut ctx = TestContext::scalar(vec![Value::Int64(3), Value::Int64(1)]);
        scalar_emit_native(&mut ctx).unwrap();
        assert_eq!(ctx.emitted().len(), 3);
        assert!(matches!(ctx.emitted()[0][0], Value::Int64(0)));
        assert!(matches!(ctx.emitted()[0][1], Value::Double(_)));
    }
}
