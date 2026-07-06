use exasol_udf_macros::exasol_udf;
use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::error::UdfError;
use exasol_udf_sdk::value::Value;

/// SET UDF: echoes its `(amount DECIMAL, event_date DATE, event_ts TIMESTAMP)`
/// input row back unchanged via the row emit path, exercising the ingest
/// fast-path parsers (`fast_parse_date`/`fast_parse_timestamp` inside
/// `decode_string_block`) end-to-end through a live DB round-trip — the
/// mirror image of `numeric_temporal_emit`'s emit-side coverage.
///
/// `ctx.get_decimal`/`ctx.get_date`/`ctx.get_timestamp` read the values
/// already decoded by the runtime's ingest path when the row was materialised
/// from the wire's string block; a decode regression would surface here as a
/// value that no longer round-trips to the DB-side literal that produced it.
///
/// EMITS schema: `amount DECIMAL(18,2), event_date DATE, event_ts TIMESTAMP`.
#[exasol_udf]
pub fn numeric_temporal_ingest(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    while ctx.next()? {
        let amount = ctx.get_decimal(0)?;
        let event_date = ctx.get_date(1)?;
        let event_ts = ctx.get_timestamp(2)?;

        ctx.emit(&[
            amount.map_or(Value::Null, Value::Numeric),
            event_date.map_or(Value::Null, Value::Date),
            event_ts.map_or(Value::Null, Value::Timestamp),
        ])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use exasol_udf_sdk::value::Decimal;

    struct TestCtx {
        rows: Vec<Vec<Value>>,
        cursor: usize,
        emitted: Vec<Vec<Value>>,
    }

    impl TestCtx {
        fn new(rows: Vec<Vec<Value>>) -> Self {
            Self {
                rows,
                cursor: 0,
                emitted: Vec::new(),
            }
        }
    }

    impl UdfContext for TestCtx {
        fn num_columns(&self) -> usize {
            self.rows.first().map_or(0, |r| r.len())
        }

        fn get(&self, col: usize) -> Result<&Value, UdfError> {
            self.rows[self.cursor - 1]
                .get(col)
                .ok_or_else(|| UdfError::User(format!("col {} out of range", col)))
        }

        fn emit(&mut self, values: &[Value]) -> Result<(), UdfError> {
            self.emitted.push(values.to_vec());
            Ok(())
        }

        fn next(&mut self) -> Result<bool, UdfError> {
            if self.cursor < self.rows.len() {
                self.cursor += 1;
                Ok(true)
            } else {
                Ok(false)
            }
        }
    }

    #[test]
    fn echoes_numeric_date_timestamp_row_unchanged() {
        let amount = Decimal {
            unscaled: 123456,
            scale: 2,
        };
        let event_date = NaiveDate::from_ymd_opt(2026, 7, 6).unwrap();
        let event_ts = event_date.and_hms_milli_opt(12, 30, 15, 250).unwrap();

        let mut ctx = TestCtx::new(vec![vec![
            Value::Numeric(amount.clone()),
            Value::Date(event_date),
            Value::Timestamp(event_ts),
        ]]);
        numeric_temporal_ingest(&mut ctx).unwrap();
        assert_eq!(
            ctx.emitted,
            vec![vec![
                Value::Numeric(amount),
                Value::Date(event_date),
                Value::Timestamp(event_ts),
            ]]
        );
    }

    #[test]
    fn echoes_multiple_rows_including_edge_dates() {
        let rows = vec![
            vec![
                Value::Numeric(Decimal {
                    unscaled: -4250,
                    scale: 2,
                }),
                Value::Date(NaiveDate::from_ymd_opt(1970, 1, 1).unwrap()),
                Value::Timestamp(
                    NaiveDate::from_ymd_opt(1999, 12, 31)
                        .unwrap()
                        .and_hms_opt(23, 59, 59)
                        .unwrap(),
                ),
            ],
            vec![
                Value::Numeric(Decimal {
                    unscaled: 0,
                    scale: 2,
                }),
                Value::Date(NaiveDate::from_ymd_opt(2000, 2, 29).unwrap()),
                Value::Timestamp(
                    NaiveDate::from_ymd_opt(2000, 2, 29)
                        .unwrap()
                        .and_hms_opt(0, 0, 0)
                        .unwrap(),
                ),
            ],
        ];

        let mut ctx = TestCtx::new(rows.clone());
        numeric_temporal_ingest(&mut ctx).unwrap();
        assert_eq!(ctx.emitted, rows);
    }

    #[test]
    fn echoes_null_row_unchanged() {
        let mut ctx = TestCtx::new(vec![vec![Value::Null, Value::Null, Value::Null]]);
        numeric_temporal_ingest(&mut ctx).unwrap();
        assert_eq!(
            ctx.emitted,
            vec![vec![Value::Null, Value::Null, Value::Null]]
        );
    }
}
