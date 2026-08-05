use crate::error::UdfError;
use crate::value::{Decimal, Value};

/// Context for a single UDF call — provided by the host, read by the UDF
pub trait UdfContext {
    /// Number of input columns
    fn num_columns(&self) -> usize;
    /// Get a specific input column value (0-indexed)
    fn get(&self, col: usize) -> Result<&Value, UdfError>;
    /// Emit one output row. Valid only for EMITS output (any number of rows per
    /// invocation); the host bans it in RETURNS output, where the returned value
    /// crosses via `set_return` instead.
    fn emit(&mut self, values: &[Value]) -> Result<(), UdfError>;
    /// Advance to the next input row of a SET group, spanning input batches;
    /// returns false at the group boundary. Valid only for SET (Multiple) input:
    /// the host bans it in scalar (ExactlyOnce) input, where the framework drives
    /// one invocation per row.
    fn next(&mut self) -> Result<bool, UdfError>;

    /// Set the single RETURNS output value for this invocation. This is the
    /// sanctioned value-return channel for RETURNS-shape UDFs; the macro's
    /// RETURNS shim calls it with the converted return value (`None` → SQL
    /// NULL). Distinct from `emit`, so the host bridge can accept the framework
    /// return while banning an author `emit()` in RETURNS output. The default
    /// reports the method as unimplemented for contexts that do not override it.
    fn set_return(&mut self, _value: Option<Value>) -> Result<(), UdfError> {
        Err(UdfError::Unimplemented("set_return".into()))
    }

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
#[path = "context_tests.rs"]
mod tests;
