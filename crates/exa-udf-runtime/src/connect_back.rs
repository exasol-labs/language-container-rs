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

use arrow::record_batch::RecordBatch;
use exa_zmq_protocol::ConnInfo;
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
    fn query_arrow(&mut self, sql: &str) -> Result<Vec<RecordBatch>, UdfError> {
        cb_log(&format!("[cb] query_arrow: '{sql}'"));
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            connect_back_rt()
                .block_on(self.inner.query(sql))
                .map_err(|e| UdfError::ConnectBack(e.to_string()))
        }));
        cb_log("[cb] query_arrow: catch_unwind done");
        match result {
            Ok(Ok(batches)) => {
                cb_log(&format!("[cb] query_arrow: ok, {} batches", batches.len()));
                Ok(batches)
            }
            Ok(Err(e)) => {
                cb_log(&format!("[cb] query_arrow: error: {e}"));
                Err(e)
            }
            Err(payload) => {
                let msg = payload
                    .downcast_ref::<&str>()
                    .copied()
                    .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
                    .unwrap_or("unknown panic payload");
                cb_log(&format!("[cb] query_arrow panic: {msg}"));
                Err(UdfError::ConnectBack(format!(
                    "panic in query_arrow: {msg}"
                )))
            }
        }
    }

    /// Override the default `query` so the arrow→`Value` conversion runs here,
    /// in the runtime's own arrow-link context. The default trait impl would
    /// run the conversion in the *caller's* (UDF `.so`'s) arrow context, where
    /// `downcast_ref` on runtime-produced arrays fails on a `TypeId` mismatch.
    /// Returning `Vec<Vec<Value>>` keeps arrow types off the FFI boundary.
    fn query(&mut self, sql: &str) -> Result<Vec<Vec<Value>>, UdfError> {
        let batches = self.query_arrow(sql)?;
        exasol_udf_sdk::connect_back::record_batches_to_rows(&batches)
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
}

impl RuntimeExaConnection {
    /// Drive an async transaction control operation to completion on the shared
    /// connect-back runtime, mapping `QueryError` to [`UdfError::ConnectBack`]
    /// and catching any panic so it cannot cross the UDF FFI boundary — the same
    /// contract as `query_arrow`/`execute`.
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
