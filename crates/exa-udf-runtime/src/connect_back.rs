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
        cb_log("[cb] drop: shutdown start");
        // Drive the async close inside the Tokio runtime so that TLS teardown
        // (SSL_shutdown, tokio::net::TcpStream deregister) happens while the
        // IO driver is live.  Errors are ignored — we're in a destructor.
        let _ = connect_back_rt().block_on(self.inner.shutdown());
        cb_log("[cb] drop: shutdown done");
    }
}

impl ExaConnection for RuntimeExaConnection {
    /// Override the default `query_for_each` so result batches are converted
    /// and consumed one at a time, in the runtime's own arrow-link context.
    ///
    /// Fetching and conversion both run here, in the runtime crate, so the
    /// per-type `downcast_ref` calls in `record_batch_to_rows` resolve against
    /// the runtime's arrow `TypeId`s. The default trait impl would run the
    /// conversion in the *caller's* (UDF `.so`'s) arrow context, where the
    /// downcast fails on a `TypeId` mismatch.
    ///
    /// The fetch is driven with a single `block_on` over
    /// `execute(sql).await?.fetch_all().await?`. We deliberately do not use
    /// `ResultSet::into_iterator()` / `next_batch()`: those call
    /// `Handle::try_current()` then `handle.block_on(...)`, which on our
    /// `current_thread` runtime would re-enter the only runtime thread from
    /// within an outer `block_on` and deadlock. `fetch_all` materialises the
    /// batches once; we then iterate the owned `Vec` with `into_iter()`,
    /// converting and dropping each batch before processing the next so a
    /// batch's arrow buffers are released before its rows are handed to `f`.
    fn query_for_each(
        &mut self,
        sql: &str,
        f: &mut dyn FnMut(Vec<Value>) -> Result<(), UdfError>,
    ) -> Result<(), UdfError> {
        cb_log(&format!("[cb] query_for_each: '{sql}'"));
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
                Ok::<Vec<_>, UdfError>(batches)
            })
        }));
        cb_log("[cb] query_for_each: fetch done");
        let batches = match result {
            Ok(Ok(batches)) => batches,
            Ok(Err(e)) => {
                cb_log(&format!("[cb] query_for_each: error: {e}"));
                return Err(e);
            }
            Err(payload) => {
                let msg = payload
                    .downcast_ref::<&str>()
                    .copied()
                    .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
                    .unwrap_or("unknown panic payload");
                cb_log(&format!("[cb] query_for_each panic: {msg}"));
                return Err(UdfError::ConnectBack(format!(
                    "panic in query_for_each: {msg}"
                )));
            }
        };
        cb_log(&format!(
            "[cb] query_for_each: ok, {} batches",
            batches.len()
        ));
        for batch in batches {
            let rows = exasol_udf_sdk::connect_back::record_batch_to_rows(&batch)?;
            drop(batch);
            for row in rows {
                f(row)?;
            }
        }
        Ok(())
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
        cb_log(&format!("[cb] execute: '{sql}'"));
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            connect_back_rt()
                .block_on(self.inner.execute_update(sql))
                .map(|rows| rows.max(0) as u64)
                .map_err(|e| UdfError::ConnectBack(e.to_string()))
        }));
        cb_log("[cb] execute: returned from block_on");
        match result {
            Ok(r) => r,
            Err(payload) => {
                let msg = payload
                    .downcast_ref::<&str>()
                    .copied()
                    .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
                    .unwrap_or("unknown panic payload");
                cb_log(&format!("[cb] execute panic: {msg}"));
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
        cb_log(&format!(
            "[cb] execute_batch: '{}', {} rows",
            sql,
            rows.len()
        ));
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
                // Log close errors but don't replace the execution result.
                if let Err(e) = self.inner.close_prepared(stmt).await {
                    cb_log(&format!("[cb] execute_batch: close_prepared error: {e}"));
                }
                count.map(|n| n.max(0) as u64)
            })
        }));
        cb_log("[cb] execute_batch: returned from block_on");
        match result {
            Ok(r) => r,
            Err(payload) => {
                let msg = payload
                    .downcast_ref::<&str>()
                    .copied()
                    .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
                    .unwrap_or("unknown panic payload");
                cb_log(&format!("[cb] execute_batch panic: {msg}"));
                Err(UdfError::ConnectBack(format!(
                    "panic in execute_batch: {msg}"
                )))
            }
        }
    }
}

impl RuntimeExaConnection {
    /// Drive an async transaction control operation to completion on the shared
    /// connect-back runtime, mapping `QueryError` to [`UdfError::ConnectBack`]
    /// and catching any panic so it cannot cross the UDF FFI boundary — the same
    /// contract as `query_for_each`/`execute`.
    fn run_txn_op<'a, F, Fut>(&'a mut self, name: &str, op: F) -> Result<(), UdfError>
    where
        F: FnOnce(&'a mut Connection) -> Fut,
        Fut: std::future::Future<Output = Result<(), exarrow_rs::error::QueryError>> + 'a,
    {
        cb_log(&format!("[cb] {name}"));
        let fut = op(&mut self.inner);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            connect_back_rt()
                .block_on(fut)
                .map_err(|e| UdfError::ConnectBack(e.to_string()))
        }));
        cb_log(&format!("[cb] {name}: returned from block_on"));
        match result {
            Ok(r) => r,
            Err(payload) => {
                let msg = payload
                    .downcast_ref::<&str>()
                    .copied()
                    .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
                    .unwrap_or("unknown panic payload");
                cb_log(&format!("[cb] {name} panic: {msg}"));
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

/// Open a new external-client session to the named-connection address.
/// Connect-back is always a new session and a new transaction — the Exasol core
/// cannot share the invoking query's transaction with a container UDF. SSL
/// verification is disabled per project rules.
fn cb_log(msg: &str) {
    use std::io::Write;
    for path in &["/tmp/cb_debug.txt"] {
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let _ = writeln!(f, "{msg}");
            return;
        }
    }
    let _ = writeln!(std::io::stderr(), "[slc-cb] {msg}");
}

pub fn open_connection(conn_info: &ConnInfo) -> Result<RuntimeExaConnection, UdfError> {
    ensure_rustls_provider();
    let dsn = build_dsn(conn_info);
    cb_log(&format!(
        "[cb] open_connection: connecting to {}",
        conn_info.address
    ));
    // Wrap in catch_unwind: panics in exarrow-rs/tokio/aws-lc-rs must not
    // cross the FFI boundary into exaudfclient (undefined behaviour).
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        cb_log("[cb] creating Driver");
        let driver = Driver::new();
        cb_log("[cb] Driver created, calling driver.open");
        let db = driver
            .open(&dsn)
            .map_err(|e| UdfError::ConnectBack(e.to_string()))?;
        cb_log("[cb] driver.open ok, calling db.connect");
        let r = connect_back_rt()
            .block_on(db.connect())
            .map_err(|e| UdfError::ConnectBack(e.to_string()));
        cb_log(&format!(
            "[cb] db.connect returned: {}",
            match &r {
                Ok(_) => "Ok".to_string(),
                Err(e) => format!("Err({e})"),
            }
        ));
        r
    }));
    cb_log("[cb] catch_unwind returned");
    match result {
        Ok(Ok(inner)) => Ok(RuntimeExaConnection { inner }),
        Ok(Err(e)) => Err(e),
        Err(payload) => {
            let msg = payload
                .downcast_ref::<&str>()
                .copied()
                .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
                .unwrap_or("unknown panic payload");
            cb_log(&format!("[cb] panic caught: {msg}"));
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
mod tests {
    use super::*;

    #[test]
    fn execute_batch_value_mapping_roundtrip() {
        // Supported variants map to the expected Parameter discriminants.
        assert!(matches!(
            value_to_parameter(&Value::Null),
            Ok(Parameter::Null)
        ));
        assert!(matches!(
            value_to_parameter(&Value::Bool(true)),
            Ok(Parameter::Boolean(true))
        ));
        assert!(matches!(
            value_to_parameter(&Value::Bool(false)),
            Ok(Parameter::Boolean(false))
        ));
        assert!(matches!(
            value_to_parameter(&Value::Int32(7)),
            Ok(Parameter::Integer(7))
        ));
        assert!(matches!(
            value_to_parameter(&Value::Int64(-1)),
            Ok(Parameter::Integer(-1))
        ));
        assert!(matches!(
            value_to_parameter(&Value::Double(1.5)),
            Ok(Parameter::Float(_))
        ));
        let s = Value::String("hello".into());
        assert!(matches!(value_to_parameter(&s), Ok(Parameter::String(_))));

        // Int32 is widened to i64.
        if let Ok(Parameter::Integer(n)) = value_to_parameter(&Value::Int32(42)) {
            assert_eq!(n, 42i64);
        } else {
            panic!("Int32 did not widen to Integer");
        }

        // Unsupported variants return Unimplemented.
        use exasol_udf_sdk::value::Decimal;
        let num = Value::Numeric(Decimal {
            unscaled: 1,
            scale: 0,
        });
        assert!(matches!(
            value_to_parameter(&num),
            Err(UdfError::Unimplemented(_))
        ));

        let d = Value::Date(chrono::NaiveDate::from_ymd_opt(2024, 1, 1).unwrap());
        assert!(matches!(
            value_to_parameter(&d),
            Err(UdfError::Unimplemented(_))
        ));

        let ts = Value::Timestamp(chrono::NaiveDateTime::default());
        assert!(matches!(
            value_to_parameter(&ts),
            Err(UdfError::Unimplemented(_))
        ));
    }

    #[test]
    fn dsn_disables_cert_validation_and_carries_credentials() {
        let info = ConnInfo {
            kind: "EXASOL".into(),
            address: "10.0.0.5:8563".into(),
            user: "sys".into(),
            password: "exasol".into(),
        };
        assert_eq!(
            build_dsn(&info),
            "exasol://sys:exasol@10.0.0.5:8563?validateservercertificate=0"
        );
    }

    /// The DSN uses `ConnInfo.address` as the host:port, not any other IP
    /// that might be available in the runtime environment (e.g. the cluster IP).
    #[test]
    fn connect_back_dsn_targets_address_as_external_client() {
        let info = ConnInfo {
            kind: "GENERIC".into(),
            address: "192.0.2.99:8563".into(),
            user: "alice".into(),
            password: "secret".into(),
        };
        let dsn = build_dsn(&info);
        assert!(
            dsn.contains("192.0.2.99"),
            "DSN must embed conn.address; got: {dsn}"
        );
    }

    /// The DSN is built solely from `ConnInfo` fields; no cluster node IP is
    /// injected. Verified by using an address different from any node IP.
    #[test]
    fn connect_back_dsn_built_only_from_connection_object() {
        let cluster_ip = "10.0.0.5"; // not in ConnInfo.address
        let info = ConnInfo {
            kind: "GENERIC".into(),
            address: "192.0.2.55:8563".into(),
            user: "bob".into(),
            password: "pass".into(),
        };
        let dsn = build_dsn(&info);
        assert!(
            !dsn.contains(cluster_ip),
            "DSN must not contain cluster IP; got: {dsn}"
        );
        assert!(
            dsn.contains("192.0.2.55"),
            "DSN must contain conn.address; got: {dsn}"
        );
    }
}
