use crate::error::UdfError;
use crate::value::{Decimal, Value};

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

    /// Maximum memory (in bytes) the host has allocated for this UDF sandbox, as reported
    /// by the DB in the handshake metadata.  Returns `0` when the host did not supply a
    /// limit or when called on a context that does not override this method.  This is plain
    /// metadata — no connect-back feature gate applies.
    fn memory_limit(&self) -> u64 {
        0
    }

    /// Session ID of the current Exasol session, from the handshake metadata.
    /// Returns `0` on a context that does not override this method.
    fn session_id(&self) -> u64 {
        0
    }

    /// Statement number within the current session, from the handshake metadata.
    /// Returns `0` on a context that does not override this method.
    fn statement_id(&self) -> u32 {
        0
    }

    /// 0-based ID of the cluster node running this UDF instance, from the
    /// handshake metadata.  Returns `0` on a context that does not override this.
    fn node_id(&self) -> u32 {
        0
    }

    /// Number of nodes in the Exasol cluster, from the handshake metadata.
    /// Returns `0` on a context that does not override this method.
    fn node_count(&self) -> u32 {
        0
    }

    /// Long unique ID of the VM / UDF process instance, from the handshake
    /// metadata.  Returns `0` on a context that does not override this method.
    fn vm_id(&self) -> u64 {
        0
    }

    /// Name of the database, from the handshake metadata.  Returns an empty
    /// string on a context that does not override this method.
    fn database_name(&self) -> String {
        String::new()
    }

    /// Version of the database, from the handshake metadata.  Returns an empty
    /// string on a context that does not override this method.
    fn database_version(&self) -> String {
        String::new()
    }

    /// Name of the running script, from the handshake metadata.  Returns an
    /// empty string on a context that does not override this method.
    fn script_name(&self) -> String {
        String::new()
    }

    /// Schema of the running script, from the handshake metadata.  Returns an
    /// empty string on a context that does not override this method.
    fn script_schema(&self) -> String {
        String::new()
    }

    /// Current user reported by the DB, from the handshake metadata.  Returns
    /// `None` when the DB did not report it (proto `optional`) or on a context
    /// that does not override this method.
    fn current_user(&self) -> Option<String> {
        None
    }

    /// Current schema reported by the DB, from the handshake metadata.  Returns
    /// `None` when the DB did not report it (proto `optional`) or on a context
    /// that does not override this method.
    fn current_schema(&self) -> Option<String> {
        None
    }

    /// Scope user reported by the DB, from the handshake metadata.  Returns
    /// `None` when the DB did not report it (proto `optional`) or on a context
    /// that does not override this method.
    fn scope_user(&self) -> Option<String> {
        None
    }

    /// The resolved verbosity level for this UDF invocation.  UDF code uses this
    /// to decide whether to write a log line via `udf_log!`.  The host bridge
    /// overrides this to return the session-level resolved by `%udf_debug_level`;
    /// the default (`INFO`) keeps existing UDFs that do not override the method
    /// compiling and behaving unchanged.
    fn debug_level(&self) -> tracing::Level {
        tracing::Level::INFO
    }

    /// Return the IP address of the cluster node that started this language container.
    /// The IP is parsed from the ZMQ endpoint; no network call is made.
    fn cluster_ip(&self) -> Result<String, UdfError> {
        Err(UdfError::Unimplemented("connect-back not available".into()))
    }

    /// Fetch raw credentials for a named Exasol CONNECTION object.
    fn connection(&self, _name: &str) -> Result<crate::connect_back::ConnectionObject, UdfError> {
        Err(UdfError::Unimplemented("connect-back not available".into()))
    }

    /// Open a live Exasol session using credentials from a `ConnectionObject`.
    fn connect_back(
        &mut self,
        _conn: &crate::connect_back::ConnectionObject,
    ) -> Result<Box<dyn crate::connect_back::ExaConnection>, UdfError> {
        Err(UdfError::Unimplemented("connect-back not available".into()))
    }

    /// Emit a RecordBatch already serialised to Arrow IPC bytes. The host
    /// deserialises and encodes it. Bytes — not Arrow types — cross the .so
    /// boundary (Arrow is not ABI-stable across the cdylib boundary; see B-002).
    /// Authors call `emit_batch` (the `EmitBatch` ext-trait), not this directly.
    fn emit_record_batch_ipc(&mut self, _ipc: &[u8]) -> Result<(), UdfError> {
        Err(UdfError::Unimplemented("emit_record_batch_ipc".into()))
    }
}

/// Ergonomic batch-emit extension for any [`UdfContext`].
///
/// The serialisation is monomorphised in the caller (UDF) crate, so the Arrow
/// `RecordBatch` never crosses the `.so` boundary — only the IPC bytes do.
#[cfg(feature = "emit-arrow")]
pub trait EmitBatch {
    /// Emit a whole Arrow `RecordBatch`. Serialised to Arrow IPC bytes in the
    /// caller crate; only the bytes cross the `.so` boundary.
    fn emit_batch(&mut self, batch: &arrow::record_batch::RecordBatch) -> Result<(), UdfError>;
}

#[cfg(feature = "emit-arrow")]
impl<C: UdfContext + ?Sized> EmitBatch for C {
    fn emit_batch(&mut self, batch: &arrow::record_batch::RecordBatch) -> Result<(), UdfError> {
        let ipc = record_batch_to_ipc(batch)?;
        self.emit_record_batch_ipc(&ipc)
    }
}

/// Serialise a single `RecordBatch` to an Arrow IPC stream (schema + one batch).
#[cfg(feature = "emit-arrow")]
fn record_batch_to_ipc(batch: &arrow::record_batch::RecordBatch) -> Result<Vec<u8>, UdfError> {
    let mut buf = Vec::new();
    {
        let mut w = arrow::ipc::writer::StreamWriter::try_new(&mut buf, &batch.schema())
            .map_err(|e| UdfError::Type(format!("emit_batch: IPC writer init: {e}")))?;
        w.write(batch)
            .map_err(|e| UdfError::Type(format!("emit_batch: IPC write: {e}")))?;
        w.finish()
            .map_err(|e| UdfError::Type(format!("emit_batch: IPC finish: {e}")))?;
    }
    Ok(buf)
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
    fn default_memory_limit_is_zero() {
        let ctx = DummyCtx;
        assert_eq!(ctx.memory_limit(), 0);
    }

    #[test]
    fn default_handshake_metadata_is_neutral() {
        let ctx = DummyCtx;
        // Numeric accessors default to 0 ("not reported").
        assert_eq!(ctx.session_id(), 0u64);
        assert_eq!(ctx.statement_id(), 0u32);
        assert_eq!(ctx.node_id(), 0u32);
        assert_eq!(ctx.node_count(), 0u32);
        assert_eq!(ctx.vm_id(), 0u64);
        // Owned-string accessors default to the empty string.
        assert_eq!(ctx.database_name(), "");
        assert_eq!(ctx.database_version(), "");
        assert_eq!(ctx.script_name(), "");
        assert_eq!(ctx.script_schema(), "");
        // Optional accessors default to None (mirroring proto `optional`).
        assert_eq!(ctx.current_user(), None);
        assert_eq!(ctx.current_schema(), None);
        assert_eq!(ctx.scope_user(), None);
    }

    #[test]
    fn default_debug_level_is_info() {
        let ctx = DummyCtx;
        assert_eq!(ctx.debug_level(), tracing::Level::INFO);
    }

    #[test]
    fn default_hooks_unimplemented() {
        let mut ctx = DummyCtx;

        let vsa = DummyUdf::virtual_schema_adapter_call(&mut ctx, "{}");
        assert!(matches!(vsa, Err(UdfError::Unimplemented(_))));

        let doc = DummyUdf::default_output_columns();
        assert!(matches!(doc, Err(UdfError::Unimplemented(_))));
    }

    #[cfg(feature = "emit-arrow")]
    #[test]
    fn default_emit_batch_unimplemented() {
        use super::EmitBatch;
        use arrow::array::Int64Array;
        use arrow::datatypes::{DataType, Field, Schema};
        use arrow::record_batch::RecordBatch;
        use std::sync::Arc;

        let schema = Arc::new(Schema::new(vec![Field::new("x", DataType::Int64, false)]));
        let array = Arc::new(Int64Array::from(vec![1i64]));
        let batch = RecordBatch::try_new(schema, vec![array]).unwrap();

        // `emit_batch` (the EmitBatch ext-trait) serialises to IPC then calls
        // the default `emit_record_batch_ipc`, which is unimplemented on a
        // context that does not override it.
        let mut ctx = DummyCtx;
        assert!(matches!(
            ctx.emit_batch(&batch),
            Err(UdfError::Unimplemented(_))
        ));
    }
}
