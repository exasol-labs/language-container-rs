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
use std::sync::OnceLock;
use tokio::runtime::Runtime as TokioRuntime;

static CONNECT_BACK_RT: OnceLock<TokioRuntime> = OnceLock::new();

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

impl ExaConnection for RuntimeExaConnection {
    fn query_arrow(&mut self, sql: &str) -> Result<Vec<RecordBatch>, UdfError> {
        let result = connect_back_rt()
            .block_on(self.inner.query(sql))
            .map_err(|e| UdfError::ConnectBack(e.to_string()))?;
        Ok(result)
    }

    fn execute(&mut self, sql: &str) -> Result<u64, UdfError> {
        let result = connect_back_rt()
            .block_on(self.inner.execute_update(sql))
            .map(|rows| rows.max(0) as u64)
            .map_err(|e| UdfError::ConnectBack(e.to_string()))?;
        Ok(result)
    }
}

/// Open a connect-back connection from the credentials surfaced during the
/// handshake. SSL verification is disabled to match the in-cluster connect-back
/// transport (the DB hands the UDF a trusted internal address).
pub fn open_connection(conn_info: &ConnInfo) -> Result<RuntimeExaConnection, UdfError> {
    let dsn = build_dsn(conn_info);
    // Wrap in catch_unwind: panics in exarrow-rs/tokio/aws-lc-rs must not
    // cross the FFI boundary into exaudfclient (undefined behaviour).
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
            Err(UdfError::ConnectBack(format!("panic: {msg}")))
        }
    }
}

/// Build the native-protocol exarrow-rs DSN from the named-connection credentials.
///
/// No `transport=` override is emitted, so exarrow-rs uses its default `native`
/// feature (the binary protocol) for the connect-back connection. SSL verification
/// is disabled because the connect-back endpoint is a trusted in-cluster address.
fn build_dsn(conn_info: &ConnInfo) -> String {
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
}
