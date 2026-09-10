//! Connect-back: a synchronous [`ExaConnection`] implemented over the async
//! exarrow-rs ADBC driver.
//!
//! The UDF runtime's main loop is synchronous (blocking ZMQ recv/send), but
//! exarrow-rs is async. A single dedicated `current_thread` Tokio runtime
//! bridges the two: every connection call is driven to completion with
//! `block_on`. The runtime is `current_thread` because the ZMQ loop is
//! single-threaded and only ever drives one connect-back call at a time, so a
//! multi-threaded reactor would add overhead with no benefit. The runtime
//! thread is a plain OS thread (never itself inside a Tokio context), so
//! `block_on` cannot trigger the "cannot block within a runtime" panic.

use exa_zmq_protocol::ConnInfo;
use exarrow_rs::Parameter;
use exarrow_rs::adbc::{Connection, Driver};
use exasol_udf_sdk::connect_back::ExaConnection;
use exasol_udf_sdk::error::UdfError;
use exasol_udf_sdk::value::Value;
use std::sync::OnceLock;
use tokio::runtime::Runtime as TokioRuntime;

static CONNECT_BACK_RT: OnceLock<TokioRuntime> = OnceLock::new();
static RUSTLS_PROVIDER: OnceLock<()> = OnceLock::new();

/// Install aws-lc-rs as the default rustls crypto provider exactly once.
/// exarrow-rs calls `rustls::ClientConfig::builder()` (no explicit provider)
/// which panics when both `aws-lc-rs` and `ring` are compiled in and no
/// process-wide default has been installed.
fn ensure_rustls_provider() {
    RUSTLS_PROVIDER.get_or_init(|| {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    });
}

/// The process-wide connect-back Tokio runtime, initialised on first use.
fn connect_back_rt() -> &'static TokioRuntime {
    CONNECT_BACK_RT.get_or_init(|| {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("connect-back runtime init")
    })
}

/// A live Exasol connection backed by the async exarrow-rs ADBC connection,
/// driven synchronously through the shared connect-back runtime.
pub struct RuntimeExaConnection {
    inner: Connection,
}

impl Drop for RuntimeExaConnection {
    fn drop(&mut self) {
        tracing::debug!("connect-back: shutdown start");
        let _ = connect_back_rt().block_on(self.inner.shutdown());
        tracing::debug!("connect-back: shutdown done");
    }
}

impl ExaConnection for RuntimeExaConnection {
    fn query_for_each(
        &mut self,
        sql: &str,
        f: &mut dyn FnMut(Vec<Value>) -> Result<(), UdfError>,
    ) -> Result<(), UdfError> {
        tracing::debug!(sql = %sql, "connect-back: query_for_each");
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            connect_back_rt().block_on(async {
                let batches = self
                    .inner
                    .execute(sql)
                    .await
                    .map_err(|e| UdfError::ConnectBack(e.to_string()))?
                    .fetch_all()
                    .await
                    .map_err(|e| UdfError::ConnectBack(e.to_string()))?;
                for batch in batches {
                    let rows = exasol_udf_sdk::connect_back::record_batch_to_rows(&batch)?;
                    drop(batch);
                    for row in rows {
                        f(row)?;
                    }
                }
                Ok(())
            })
        }));
        match result {
            Ok(r) => r,
            Err(payload) => {
                let msg = payload
                    .downcast_ref::<&str>()
                    .copied()
                    .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
                    .unwrap_or("unknown panic payload");
                tracing::debug!(msg, "connect-back: query_for_each panic");
                Err(UdfError::ConnectBack(format!(
                    "panic in query_for_each: {msg}"
                )))
            }
        }
    }

    /// Override the default `query` so the arrow→`Value` conversion runs here,
    /// in the runtime's own arrow-link context, by delegating to
    /// [`RuntimeExaConnection::query_for_each`] and collecting its rows.
    /// Returning `Vec<Vec<Value>>` keeps arrow types off the FFI boundary.
    fn query(&mut self, sql: &str) -> Result<Vec<Vec<Value>>, UdfError> {
        let mut rows = Vec::new();
        self.query_for_each(sql, &mut |row| {
            rows.push(row);
            Ok(())
        })?;
        Ok(rows)
    }

    fn execute(&mut self, sql: &str) -> Result<u64, UdfError> {
        tracing::debug!(sql = %sql, "connect-back: execute");
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            connect_back_rt()
                .block_on(self.inner.execute_update(sql))
                .map(|rows| rows.max(0) as u64)
                .map_err(|e| UdfError::ConnectBack(e.to_string()))
        }));
        match result {
            Ok(r) => r,
            Err(payload) => {
                let msg = payload
                    .downcast_ref::<&str>()
                    .copied()
                    .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
                    .unwrap_or("unknown panic payload");
                tracing::debug!(msg, "connect-back: execute panic");
                Err(UdfError::ConnectBack(format!("panic in execute: {msg}")))
            }
        }
    }

    fn begin(&mut self) -> Result<(), UdfError> {
        self.run_txn_op("begin", |inner| inner.begin_transaction())
    }

    fn commit(&mut self) -> Result<(), UdfError> {
        self.run_txn_op("commit", |inner| inner.commit())
    }

    fn rollback(&mut self) -> Result<(), UdfError> {
        self.run_txn_op("rollback", |inner| inner.rollback())
    }

    fn execute_batch(&mut self, sql: &str, rows: &[Vec<Value>]) -> Result<u64, UdfError> {
        if rows.is_empty() {
            return Ok(0);
        }
        tracing::debug!(sql = %sql, rows = rows.len(), "connect-back: execute_batch");
        let param_rows: Vec<Vec<Parameter>> = rows
            .iter()
            .map(|row| row.iter().map(value_to_parameter).collect::<Result<_, _>>())
            .collect::<Result<_, _>>()?;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            connect_back_rt().block_on(async {
                let stmt = self
                    .inner
                    .prepare(sql)
                    .await
                    .map_err(|e| UdfError::ConnectBack(e.to_string()))?;
                let count = self
                    .inner
                    .execute_batch_update(&stmt, &param_rows)
                    .await
                    .map_err(|e| UdfError::ConnectBack(e.to_string()));
                if let Err(e) = self.inner.close_prepared(stmt).await {
                    tracing::debug!(error = %e, "connect-back: close_prepared error");
                }
                count.map(|n| n.max(0) as u64)
            })
        }));
        match result {
            Ok(r) => r,
            Err(payload) => {
                let msg = payload
                    .downcast_ref::<&str>()
                    .copied()
                    .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
                    .unwrap_or("unknown panic payload");
                tracing::debug!(msg, "connect-back: execute_batch panic");
                Err(UdfError::ConnectBack(format!(
                    "panic in execute_batch: {msg}"
                )))
            }
        }
    }
}

impl RuntimeExaConnection {
    fn run_txn_op<'a, F, Fut>(&'a mut self, name: &str, op: F) -> Result<(), UdfError>
    where
        F: FnOnce(&'a mut Connection) -> Fut,
        Fut: std::future::Future<Output = Result<(), exarrow_rs::error::QueryError>> + 'a,
    {
        tracing::debug!(op = name, "connect-back: txn");
        let fut = op(&mut self.inner);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            connect_back_rt()
                .block_on(fut)
                .map_err(|e| UdfError::ConnectBack(e.to_string()))
        }));
        match result {
            Ok(r) => r,
            Err(payload) => {
                let msg = payload
                    .downcast_ref::<&str>()
                    .copied()
                    .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
                    .unwrap_or("unknown panic payload");
                tracing::debug!(op = name, msg, "connect-back: txn panic");
                Err(UdfError::ConnectBack(format!("panic in {name}: {msg}")))
            }
        }
    }
}

/// Map one SDK [`Value`] to the exarrow-rs [`Parameter`] required by prepared
/// statement execution.
///
/// The common DML binding types (String, integers, float, boolean, null) map
/// directly. Numeric/Date/Timestamp have no lossless wire mapping today and
/// return [`UdfError::Unimplemented`] — callers that need them can format the
/// value as a string literal and use `execute` instead.
fn value_to_parameter(v: &Value) -> Result<Parameter, UdfError> {
    match v {
        Value::Null => Ok(Parameter::Null),
        Value::Bool(b) => Ok(Parameter::Boolean(*b)),
        Value::Int32(i) => Ok(Parameter::Integer(*i as i64)),
        Value::Int64(i) => Ok(Parameter::Integer(*i)),
        Value::Double(f) => Ok(Parameter::Float(*f)),
        Value::String(s) => Ok(Parameter::String(s.clone())),
        other => Err(UdfError::Unimplemented(format!(
            "execute_batch: no Parameter mapping for {other:?}"
        ))),
    }
}

pub fn open_connection(conn_info: &ConnInfo) -> Result<RuntimeExaConnection, UdfError> {
    ensure_rustls_provider();
    let dsn = build_dsn(conn_info);
    tracing::debug!(address = %conn_info.address, "connect-back: connecting");
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let driver = Driver::new();
        let db = driver
            .open(&dsn)
            .map_err(|e| UdfError::ConnectBack(e.to_string()))?;
        connect_back_rt()
            .block_on(db.connect())
            .map_err(|e| UdfError::ConnectBack(e.to_string()))
    }));
    match result {
        Ok(Ok(inner)) => Ok(RuntimeExaConnection { inner }),
        Ok(Err(e)) => Err(e),
        Err(payload) => {
            let msg = payload
                .downcast_ref::<&str>()
                .copied()
                .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
                .unwrap_or("unknown panic payload");
            tracing::debug!(msg, "connect-back: open_connection panic");
            Err(UdfError::ConnectBack(format!("panic: {msg}")))
        }
    }
}

fn build_dsn(conn_info: &ConnInfo) -> String {
    // Use the native binary protocol (no transport=websocket). The WebSocket
    // transport sends a proper WS close frame on disconnect, which triggers
    // Exasol's safeDisconnectTimeout (10 s) + SO_LINGER (1 s) before the
    // connect-back exasql process (Part:44) exits. Part:40 waits for Part:44
    // to deregister before sending MT_CLEANUP, so the 11 s delay causes
    // Part:40's TimerWatchDog to fire SIGABRT.
    //
    // The native protocol sends CMD_DISCONNECT then drops the TCP stream
    // immediately (self.stream = None) without a WS close frame — matching
    // PyExasol's close() behavior and making Part:44 deregister in < 1 s.
    format!(
        "exasol://{}:{}@{}?validateservercertificate=0",
        conn_info.user, conn_info.password, conn_info.address
    )
}

#[cfg(test)]
#[path = "connect_back_tests.rs"]
mod tests;
