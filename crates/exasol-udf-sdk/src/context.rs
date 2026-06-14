use crate::error::UdfError;
use crate::value::{Decimal, ExaType, Value};

/// Context for a single UDF call — provided by the host, read by the UDF
pub trait UdfContext {
    /// Number of input columns
    fn num_columns(&self) -> usize;
    /// Get a specific input column value (0-indexed)
    fn get(&self, col: usize) -> Result<&Value, UdfError>;
    /// Emit one output row. For scalar: called once per input row. For set/EMITS: called per output row.
    fn emit(&mut self, values: &[Value]) -> Result<(), UdfError>;
    /// Advance to the next input row (set UDFs only). Returns false when exhausted.
    fn next(&mut self) -> Result<bool, UdfError>;

    /// Get a column value, mapping SQL NULL to `None`.
    fn get_value(&self, col: usize) -> Result<Option<Value>, UdfError> {
        match self.get(col)? {
            Value::Null => Ok(None),
            v => Ok(Some(v.clone())),
        }
    }

    /// Get a column as `i64`. Accepts integral `Numeric` (scale 0); errors on a fractional part.
    fn get_i64(&self, col: usize) -> Result<Option<i64>, UdfError> {
        match self.get(col)? {
            Value::Null => Ok(None),
            Value::Int64(i) => Ok(Some(*i)),
            Value::Int32(i) => Ok(Some(*i as i64)),
            Value::Numeric(d) => {
                if d.scale == 0 {
                    d.unscaled
                        .try_into()
                        .map(Some)
                        .map_err(|_| UdfError::Type(format!("Numeric value {} overflows i64", d)))
                } else {
                    Err(UdfError::Type(format!(
                        "Numeric value {} has non-zero scale; use get_decimal",
                        d
                    )))
                }
            }
            other => Err(UdfError::Type(format!("expected i64, got {:?}", other))),
        }
    }

    /// Get a column as `f64`. Strict: no integer coercion.
    fn get_f64(&self, col: usize) -> Result<Option<f64>, UdfError> {
        match self.get(col)? {
            Value::Null => Ok(None),
            Value::Double(f) => Ok(Some(*f)),
            other => Err(UdfError::Type(format!("expected f64, got {:?}", other))),
        }
    }

    /// Get a column as a string slice.
    fn get_string(&self, col: usize) -> Result<Option<&str>, UdfError> {
        match self.get(col)? {
            Value::Null => Ok(None),
            Value::String(s) => Ok(Some(s.as_str())),
            other => Err(UdfError::Type(format!("expected string, got {:?}", other))),
        }
    }

    /// Get a column as `bool`.
    fn get_bool(&self, col: usize) -> Result<Option<bool>, UdfError> {
        match self.get(col)? {
            Value::Null => Ok(None),
            Value::Bool(b) => Ok(Some(*b)),
            other => Err(UdfError::Type(format!("expected bool, got {:?}", other))),
        }
    }

    /// Get a column as a fixed-point `Decimal`.
    fn get_decimal(&self, col: usize) -> Result<Option<Decimal>, UdfError> {
        match self.get(col)? {
            Value::Null => Ok(None),
            Value::Numeric(d) => Ok(Some(d.clone())),
            other => Err(UdfError::Type(format!("expected decimal, got {:?}", other))),
        }
    }

    /// Get a column as a `NaiveDate`.
    fn get_date(&self, col: usize) -> Result<Option<chrono::NaiveDate>, UdfError> {
        match self.get(col)? {
            Value::Null => Ok(None),
            Value::Date(d) => Ok(Some(*d)),
            other => Err(UdfError::Type(format!("expected date, got {:?}", other))),
        }
    }

    /// Get a column as a `NaiveDateTime`.
    fn get_timestamp(&self, col: usize) -> Result<Option<chrono::NaiveDateTime>, UdfError> {
        match self.get(col)? {
            Value::Null => Ok(None),
            Value::Timestamp(ts) => Ok(Some(*ts)),
            other => Err(UdfError::Type(format!(
                "expected timestamp, got {:?}",
                other
            ))),
        }
    }

    /// Number of columns (defaults to `num_columns`).
    fn column_count(&self) -> usize {
        self.num_columns()
    }

    /// Name of a column by index.
    fn column_name(&self, col: usize) -> Result<&str, UdfError> {
        let _ = col;
        Err(UdfError::Unimplemented("column_name".into()))
    }

    /// Declared type of a column by index.
    fn column_type(&self, col: usize) -> Result<ExaType, UdfError> {
        let _ = col;
        Err(UdfError::Unimplemented("column_type".into()))
    }

    /// Index of a column by name.
    fn column_index(&self, name: &str) -> Result<usize, UdfError> {
        let _ = name;
        Err(UdfError::Unimplemented("column_index".into()))
    }

    /// Reset iteration to the first input row (set UDFs only).
    fn reset(&mut self) -> Result<(), UdfError> {
        Err(UdfError::Unimplemented("reset".into()))
    }

    /// Return the IP address of the cluster node that started this language container.
    /// The IP is parsed from the ZMQ endpoint; no network call is made.
    #[cfg(feature = "connect-back")]
    fn cluster_ip(&self) -> Result<String, UdfError> {
        Err(UdfError::Unimplemented("connect-back not available".into()))
    }

    /// Fetch raw credentials for a named Exasol CONNECTION object.
    #[cfg(feature = "connect-back")]
    fn connection(&self, _name: &str) -> Result<crate::connect_back::ConnectionObject, UdfError> {
        Err(UdfError::Unimplemented("connect-back not available".into()))
    }

    /// Open a live Exasol session using credentials from a `ConnectionObject`.
    #[cfg(feature = "connect-back")]
    fn connect_back(
        &mut self,
        _conn: &crate::connect_back::ConnectionObject,
    ) -> Result<Box<dyn crate::connect_back::ExaConnection>, UdfError> {
        Err(UdfError::Unimplemented("connect-back not available".into()))
    }
}

/// Per-call lifecycle hooks — default implementations return Unimplemented for v1 single-call hooks
pub trait UdfRun: Sized {
    fn run(ctx: &mut dyn UdfContext) -> Result<(), UdfError>;

    /// Called once before run() — default: Unimplemented
    fn virtual_schema_adapter_call(
        _ctx: &mut dyn UdfContext,
        _json_arg: &str,
    ) -> Result<String, UdfError> {
        Err(UdfError::Unimplemented(
            "virtual_schema_adapter_call".into(),
        ))
    }
    /// Called once before run() — default: Unimplemented
    fn default_output_columns() -> Result<String, UdfError> {
        Err(UdfError::Unimplemented("default_output_columns".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct DummyCtx;

    impl UdfContext for DummyCtx {
        fn num_columns(&self) -> usize {
            0
        }
        fn get(&self, _col: usize) -> Result<&Value, UdfError> {
            Err(UdfError::Type("no columns".into()))
        }
        fn emit(&mut self, _values: &[Value]) -> Result<(), UdfError> {
            Ok(())
        }
        fn next(&mut self) -> Result<bool, UdfError> {
            Ok(false)
        }
    }

    struct DummyUdf;

    impl UdfRun for DummyUdf {
        fn run(_ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
            Ok(())
        }
    }

    struct TypedDummyCtx {
        values: Vec<Value>,
    }

    impl UdfContext for TypedDummyCtx {
        fn num_columns(&self) -> usize {
            self.values.len()
        }
        fn get(&self, col: usize) -> Result<&Value, UdfError> {
            self.values
                .get(col)
                .ok_or_else(|| UdfError::Type("out of range".into()))
        }
        fn emit(&mut self, _values: &[Value]) -> Result<(), UdfError> {
            Ok(())
        }
        fn next(&mut self) -> Result<bool, UdfError> {
            Ok(false)
        }
    }

    #[test]
    fn bridge_typed_getters_return_typed_options() {
        let date = chrono::NaiveDate::from_ymd_opt(2026, 6, 14).unwrap();
        let ctx = TypedDummyCtx {
            values: vec![
                Value::Int64(42),
                Value::Numeric(Decimal {
                    unscaled: 100,
                    scale: 0,
                }),
                Value::Numeric(Decimal {
                    unscaled: 15,
                    scale: 1,
                }),
                Value::Date(date),
                Value::Null,
                Value::Int64(1),
            ],
        };

        assert_eq!(ctx.get_i64(0).unwrap(), Some(42));
        assert_eq!(ctx.get_i64(1).unwrap(), Some(100));
        assert!(matches!(ctx.get_i64(2), Err(UdfError::Type(_))));

        let expected = Decimal {
            unscaled: 15,
            scale: 1,
        };
        assert_eq!(ctx.get_decimal(2).unwrap(), Some(expected));

        assert_eq!(ctx.get_date(3).unwrap(), Some(date));
        assert_eq!(ctx.get_value(4).unwrap(), None);
        assert!(matches!(ctx.get_f64(5), Err(UdfError::Type(_))));
    }

    #[test]
    fn default_hooks_unimplemented() {
        let mut ctx = DummyCtx;

        let vsa = DummyUdf::virtual_schema_adapter_call(&mut ctx, "{}");
        assert!(matches!(vsa, Err(UdfError::Unimplemented(_))));

        let doc = DummyUdf::default_output_columns();
        assert!(matches!(doc, Err(UdfError::Unimplemented(_))));
    }
}
