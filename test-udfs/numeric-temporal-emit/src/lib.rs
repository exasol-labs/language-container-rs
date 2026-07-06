use chrono::NaiveDate;
use exasol_udf_macros::exasol_udf;
use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::error::UdfError;
use exasol_udf_sdk::value::{Decimal, Value};

/// SET UDF: drains all input rows then emits three fixed rows of
/// NUMERIC/DATE/TIMESTAMP values via the row emit path (`ctx.emit`), exercising
/// the string-block fast formatters end-to-end through a live DB round-trip.
///
/// EMITS schema: `amount DECIMAL(18,2), event_date DATE, event_ts TIMESTAMP`.
///
/// The three rows deliberately span fast-path edges: a signed/zero/scaled
/// DECIMAL, an epoch/leap-day/year-boundary DATE, and midnight / sub-second /
/// second-boundary TIMESTAMP values. Plain `TIMESTAMP` is precision-3, so the
/// only sub-second component used (`.250`) survives engine truncation.
#[exasol_udf]
pub fn numeric_temporal_emit(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    while ctx.next()? {}

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    let rows = [
        (
            Decimal {
                unscaled: 123456,
                scale: 2,
            },
            date(2026, 7, 6),
            date(2026, 7, 6).and_hms_milli_opt(12, 30, 15, 250).unwrap(),
        ),
        (
            Decimal {
                unscaled: -4250,
                scale: 2,
            },
            date(1970, 1, 1),
            date(1999, 12, 31).and_hms_opt(23, 59, 59).unwrap(),
        ),
        (
            Decimal {
                unscaled: 0,
                scale: 2,
            },
            date(2000, 2, 29),
            date(2000, 2, 29).and_hms_opt(0, 0, 0).unwrap(),
        ),
    ];

    for (amount, event_date, event_ts) in rows {
        ctx.emit(&[
            Value::Numeric(amount),
            Value::Date(event_date),
            Value::Timestamp(event_ts),
        ])?;
    }
    Ok(())
}
