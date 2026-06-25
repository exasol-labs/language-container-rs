use crate::error::UdfError;
use crate::value::Value;

#[cfg(feature = "emit-arrow")]
use crate::value::Decimal;
#[cfg(feature = "emit-arrow")]
use arrow::array::{
    Array, BooleanArray, Date32Array, Decimal128Array, Float64Array, Int32Array, Int64Array,
    LargeStringArray, StringArray, TimestampMicrosecondArray, TimestampMillisecondArray,
    TimestampNanosecondArray, TimestampSecondArray,
};
#[cfg(feature = "emit-arrow")]
use arrow::datatypes::{DataType, TimeUnit};
#[cfg(feature = "emit-arrow")]
use arrow::record_batch::RecordBatch;

/// Credentials for a named Exasol CONNECTION object or any external system.
#[derive(Debug, Clone)]
pub struct ConnectionObject {
    pub kind: String,
    pub address: String,
    pub user: String,
    pub password: String,
}

/// A live Exasol connection the UDF can use for queries and DML.
///
/// The trait is object-safe so the runtime can hand back a
/// `Box<dyn ExaConnection>`; the `Send` bound lets that box move across the
/// call boundaries the runtime manages.
///
/// Only `Vec<Value>` row data crosses the `.so` boundary — no Arrow types.
/// The `query_for_each` method is required; `query` provides a default that
/// collects all rows by delegating to it.
pub trait ExaConnection: Send {
    /// Run a query and invoke `f` for each result row.
    ///
    /// Rows are delivered in batch-then-row order. Iteration stops immediately
    /// if `f` returns an error, and that error is propagated to the caller.
    ///
    /// The callback is taken as `&mut dyn FnMut` so the method is object-safe
    /// and usable on `Box<dyn ExaConnection>`.
    fn query_for_each(
        &mut self,
        sql: &str,
        f: &mut dyn FnMut(Vec<Value>) -> Result<(), UdfError>,
    ) -> Result<(), UdfError>;

    /// Run a query and return its rows as the SDK's own [`Value`] type.
    ///
    /// This is the FFI-safe query API for UDFs: only plain `Value` data
    /// crosses the `.so` boundary (no arrow types, no `TypeId` downcasts).
    ///
    /// The default collects all rows via [`ExaConnection::query_for_each`].
    fn query(&mut self, sql: &str) -> Result<Vec<Vec<Value>>, UdfError> {
        let mut rows = Vec::new();
        self.query_for_each(sql, &mut |row| {
            rows.push(row);
            Ok(())
        })?;
        Ok(rows)
    }

    /// Execute a DML/DDL statement, returning the affected row count.
    fn execute(&mut self, sql: &str) -> Result<u64, UdfError>;

    /// Execute a parameterised DML statement once per row in `rows`, returning
    /// the total affected row count.
    ///
    /// The default returns [`UdfError::Unimplemented`] so existing mock
    /// implementations in unit tests keep compiling without changes.
    fn execute_batch(&mut self, _sql: &str, _rows: &[Vec<Value>]) -> Result<u64, UdfError> {
        Err(UdfError::Unimplemented(
            "execute_batch not supported on this connection".into(),
        ))
    }

    /// Begin an explicit transaction (disable autocommit).
    ///
    /// The default returns [`UdfError::Unimplemented`] so connections that do
    /// not manage transactions (e.g. mocks in unit tests) keep compiling.
    fn begin(&mut self) -> Result<(), UdfError> {
        Err(UdfError::Unimplemented(
            "begin not supported on this connection".into(),
        ))
    }

    /// Commit the active transaction.
    ///
    /// The default returns [`UdfError::Unimplemented`] so connections that do
    /// not manage transactions (e.g. mocks in unit tests) keep compiling.
    fn commit(&mut self) -> Result<(), UdfError> {
        Err(UdfError::Unimplemented(
            "commit not supported on this connection".into(),
        ))
    }

    /// Roll back the active transaction.
    ///
    /// The default returns [`UdfError::Unimplemented`] so connections that do
    /// not manage transactions (e.g. mocks in unit tests) keep compiling.
    fn rollback(&mut self) -> Result<(), UdfError> {
        Err(UdfError::Unimplemented(
            "rollback not supported on this connection".into(),
        ))
    }
}

/// Convert a single arrow [`RecordBatch`] into rows of the SDK's [`Value`] type.
///
/// This must be called in the same arrow-link context that produced the batch,
/// so the per-type `downcast_ref` calls resolve consistently. The resulting
/// `Vec<Vec<Value>>` is plain owned data and crosses the UDF FFI boundary safely.
///
/// Type mapping mirrors the input-row convention: Exasol `DECIMAL`/`BIGINT`
/// (arrow `Decimal128`) becomes a typed [`Value::Numeric`] carrying the unscaled
/// integer plus scale, matching how NUMERIC input columns are delivered.
#[cfg(feature = "emit-arrow")]
pub fn record_batch_to_rows(batch: &RecordBatch) -> Result<Vec<Vec<Value>>, UdfError> {
    let n_rows = batch.num_rows();
    let n_cols = batch.num_columns();
    let mut rows = Vec::with_capacity(n_rows);
    for r in 0..n_rows {
        let mut row = Vec::with_capacity(n_cols);
        for c in 0..n_cols {
            row.push(cell_to_value(batch.column(c).as_ref(), r)?);
        }
        rows.push(row);
    }
    Ok(rows)
}

/// Convert arrow record batches into rows of the SDK's [`Value`] type.
///
/// This must be called in the same arrow-link context that produced the
/// batches (the runtime), so the per-type `downcast_ref` calls resolve. The
/// resulting `Vec<Vec<Value>>` is plain owned data and crosses the UDF FFI
/// boundary safely.
#[cfg(feature = "emit-arrow")]
pub fn record_batches_to_rows(batches: &[RecordBatch]) -> Result<Vec<Vec<Value>>, UdfError> {
    let mut rows = Vec::new();
    for batch in batches {
        rows.extend(record_batch_to_rows(batch)?);
    }
    Ok(rows)
}

/// Convert one arrow array cell to a [`Value`].
#[cfg(feature = "emit-arrow")]
fn cell_to_value(col: &dyn Array, row: usize) -> Result<Value, UdfError> {
    if col.is_null(row) {
        return Ok(Value::Null);
    }
    let unexpected = |dt: &DataType| UdfError::ConnectBack(format!("unexpected arrow type {dt:?}"));
    match col.data_type() {
        DataType::Boolean => Ok(Value::Bool(
            col.as_any()
                .downcast_ref::<BooleanArray>()
                .ok_or_else(|| unexpected(col.data_type()))?
                .value(row),
        )),
        DataType::Int32 => Ok(Value::Int32(
            col.as_any()
                .downcast_ref::<Int32Array>()
                .ok_or_else(|| unexpected(col.data_type()))?
                .value(row),
        )),
        DataType::Int64 => Ok(Value::Int64(
            col.as_any()
                .downcast_ref::<Int64Array>()
                .ok_or_else(|| unexpected(col.data_type()))?
                .value(row),
        )),
        DataType::Float64 => Ok(Value::Double(
            col.as_any()
                .downcast_ref::<Float64Array>()
                .ok_or_else(|| unexpected(col.data_type()))?
                .value(row),
        )),
        DataType::Decimal128(_, _) => {
            let arr = col
                .as_any()
                .downcast_ref::<Decimal128Array>()
                .ok_or_else(|| unexpected(col.data_type()))?;
            // Exasol DECIMAL scale never exceeds 36, so the i8 arrow scale fits
            // u8 without loss. Carrying the unscaled i128 + scale round-trips the
            // value losslessly, unlike the previous decimal-string rendering.
            Ok(Value::Numeric(Decimal {
                unscaled: arr.value(row),
                scale: arr.scale() as u8,
            }))
        }
        DataType::Utf8 => Ok(Value::String(
            col.as_any()
                .downcast_ref::<StringArray>()
                .ok_or_else(|| unexpected(col.data_type()))?
                .value(row)
                .to_string(),
        )),
        DataType::LargeUtf8 => Ok(Value::String(
            col.as_any()
                .downcast_ref::<LargeStringArray>()
                .ok_or_else(|| unexpected(col.data_type()))?
                .value(row)
                .to_string(),
        )),
        DataType::Date32 => {
            let a = col
                .as_any()
                .downcast_ref::<Date32Array>()
                .ok_or_else(|| unexpected(col.data_type()))?;
            let d = a
                .value_as_date(row)
                .ok_or_else(|| UdfError::ConnectBack("invalid date value".into()))?;
            Ok(Value::Date(d))
        }
        DataType::Timestamp(unit, _) => Ok(Value::Timestamp(timestamp_cell(col, row, *unit)?)),
        other => {
            // Anything else (intervals, geometry, …) is rendered to its textual
            // form so the UDF still receives a usable value.
            let opts = arrow::util::display::FormatOptions::default();
            let fmt = arrow::util::display::ArrayFormatter::try_new(col, &opts)
                .map_err(|e| UdfError::ConnectBack(format!("formatting {other:?}: {e}")))?;
            Ok(Value::String(fmt.value(row).to_string()))
        }
    }
}

/// Decode one arrow timestamp cell into a `NaiveDateTime`.
///
/// Arrow timestamps are an `i64` count of `unit`s since the Unix epoch; we
/// reinterpret them as wall-clock UTC (`naive_utc`) because Exasol's TIMESTAMP
/// is timezone-naive. An out-of-range epoch value is a corrupt cell.
#[cfg(feature = "emit-arrow")]
fn timestamp_cell(
    col: &dyn Array,
    row: usize,
    unit: TimeUnit,
) -> Result<chrono::NaiveDateTime, UdfError> {
    let unexpected = |dt: &DataType| UdfError::ConnectBack(format!("unexpected arrow type {dt:?}"));
    let raw = match unit {
        TimeUnit::Second => col
            .as_any()
            .downcast_ref::<TimestampSecondArray>()
            .ok_or_else(|| unexpected(col.data_type()))?
            .value(row),
        TimeUnit::Millisecond => col
            .as_any()
            .downcast_ref::<TimestampMillisecondArray>()
            .ok_or_else(|| unexpected(col.data_type()))?
            .value(row),
        TimeUnit::Microsecond => col
            .as_any()
            .downcast_ref::<TimestampMicrosecondArray>()
            .ok_or_else(|| unexpected(col.data_type()))?
            .value(row),
        TimeUnit::Nanosecond => col
            .as_any()
            .downcast_ref::<TimestampNanosecondArray>()
            .ok_or_else(|| unexpected(col.data_type()))?
            .value(row),
    };
    let dt = match unit {
        TimeUnit::Second => chrono::DateTime::from_timestamp(raw, 0),
        TimeUnit::Millisecond => chrono::DateTime::from_timestamp_millis(raw),
        TimeUnit::Microsecond => chrono::DateTime::from_timestamp_micros(raw),
        TimeUnit::Nanosecond => Some(chrono::DateTime::from_timestamp_nanos(raw)),
    };
    dt.map(|dt| dt.naive_utc())
        .ok_or_else(|| UdfError::ConnectBack(format!("timestamp out of range: {raw} {unit:?}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_object_fields_public_unconditional() {
        // ConnectionObject must be constructible and readable without any feature gate.
        let obj = ConnectionObject {
            kind: "EXA".into(),
            address: "192.0.2.1:8563".into(),
            user: "sys".into(),
            password: "secret".into(),
        };
        assert_eq!(obj.kind, "EXA");
        assert_eq!(obj.address, "192.0.2.1:8563");
        assert_eq!(obj.user, "sys");
        assert_eq!(obj.password, "secret");
    }
}
