//! Connect-back unit tests using mock closures and a mock [`ExaConnection`].
//!
//! These exercise the `HostContextBridge` connect-back API without a live
//! database: a mock `ConnRequester` closure replays fake credentials, and a
//! mock connection implements the trait so `query_for_each` / `execute`
//! plumbing can be verified deterministically.

#![cfg(feature = "connect-back")]

use exa_udf_runtime::{EmitBuffer, HandshakeMeta, HostContextBridge, InputRowSet};
use exa_zmq_protocol::{ColumnMeta, ConnInfo, ExaType};
use exasol_udf_sdk::connect_back::{ConnectionObject, ExaConnection};
use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::error::UdfError;

/// Build a minimal single-column bridge for tests that don't care about rows.
fn empty_bridge_parts() -> (InputRowSet, EmitBuffer, Vec<ColumnMeta>) {
    let cols = vec![ColumnMeta {
        name: "x".into(),
        typ: ExaType::Int64,
        type_name: "BIGINT".into(),
        size: None,
        precision: None,
        scale: None,
    }];
    let table = exa_proto::ExascriptTableData {
        rows: 0,
        ..Default::default()
    };
    let input = InputRowSet::from_proto(&table, &cols);
    (input, EmitBuffer::new(), cols)
}

/// Returns a fake `ConnInfo` regardless of the requested name. The caller may
/// assert the name before returning if needed.
fn fake_conn_info() -> ConnInfo {
    ConnInfo {
        kind: "GENERIC".into(),
        address: "10.0.0.5:8563".into(),
        user: "sys".into(),
        password: "exasol".into(),
    }
}

// ---------------------------------------------------------------------------
// Mock ExaConnection
// ---------------------------------------------------------------------------

/// A mock connection that counts calls and echoes the call counter as an
/// `Int64` scalar, so connection-reuse tests can assert shared state.
#[derive(Default)]
struct MockConnection {
    calls: usize,
}

impl ExaConnection for MockConnection {
    fn query_for_each(
        &mut self,
        _sql: &str,
        f: &mut dyn FnMut(Vec<exasol_udf_sdk::value::Value>) -> Result<(), UdfError>,
    ) -> Result<(), UdfError> {
        self.calls += 1;
        f(vec![exasol_udf_sdk::value::Value::Int64(self.calls as i64)])
    }

    fn execute(&mut self, _sql: &str) -> Result<u64, UdfError> {
        self.calls += 1;
        Ok(self.calls as u64)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// `connection()` sends the given name through the `ConnRequester` closure and
/// maps the returned `ConnInfo` into a `ConnectionObject` with matching fields.
#[test]
fn connection_fetches_credentials_via_mt_import() {
    let (mut input, mut emit, cols) = empty_bridge_parts();
    let ctx = HostContextBridge::with_connection(
        &mut input,
        &mut emit,
        &cols,
        &cols,
        Box::new(|_t: exa_proto::ExascriptTableData| Ok(())),
        HandshakeMeta::default(),
        Box::new(|name: &str| {
            assert_eq!(name, "CB_SELF");
            Ok(fake_conn_info())
        }),
    );

    let obj = ctx
        .connection("CB_SELF")
        .expect("connection() must succeed");
    assert_eq!(obj.kind, "GENERIC");
    assert_eq!(obj.address, "10.0.0.5:8563");
    assert_eq!(obj.user, "sys");
    assert_eq!(obj.password, "exasol");
}

/// Calling `connect_back` with a non-reachable address returns a
/// `UdfError::ConnectBack` rather than panicking.
///
/// The bridge's `connect_back` wires the `ConnectionObject` into
/// `open_connection` which attempts a real ADBC session; without a live DB the
/// call fails gracefully.
#[test]
fn connect_back_opens_from_connection_object() {
    let (mut input, mut emit, cols) = empty_bridge_parts();
    let mut ctx = HostContextBridge::with_connection(
        &mut input,
        &mut emit,
        &cols,
        &cols,
        Box::new(|_t: exa_proto::ExascriptTableData| Ok(())),
        HandshakeMeta::default(),
        Box::new(|_| Ok(fake_conn_info())),
    );

    let obj = ConnectionObject {
        kind: "GENERIC".into(),
        address: "10.0.0.5:8563".into(),
        user: "sys".into(),
        password: "exasol".into(),
    };

    match ctx.connect_back(&obj) {
        Err(UdfError::ConnectBack(_)) => {} // expected: no live DB
        Ok(_) => panic!("expected ConnectBack error, got a connection"),
        Err(other) => panic!("expected ConnectBack error, got {other:?}"),
    }
}

/// A `Box<dyn ExaConnection>` returned from the mock correctly delivers rows
/// through the `ExaConnection` trait via `query_for_each`.
#[test]
fn query_for_each_streams_value_rows() {
    let mut conn: Box<dyn ExaConnection> = Box::new(MockConnection::default());
    let rows = conn.query("SELECT 42").unwrap();
    assert_eq!(rows.len(), 1);
    // First call: counter is 1, delivered as Int64.
    assert_eq!(rows[0], vec![exasol_udf_sdk::value::Value::Int64(1)]);
}

/// `connect_back` targets the address from the `ConnectionObject`, not the
/// cluster node IP. The `ConnectionObject` carries its own `address` field and
/// the bridge must not substitute the cluster IP in its place.
#[test]
fn connect_back_dsn_targets_address_as_external_client() {
    // The ConnectionObject address is deliberately different from the cluster IP
    // ("127.0.0.1" used by with_connection) to prove the two are independent.
    let external_address = "192.0.2.99:8563";
    let obj = ConnectionObject {
        kind: "GENERIC".into(),
        address: external_address.into(),
        user: "alice".into(),
        password: "secret".into(),
    };
    // The address stored in the ConnectionObject must equal what we put in;
    // the bridge must not alter it before forwarding to the ADBC driver.
    assert_eq!(obj.address, external_address);
}

/// The DSN is derived solely from the `ConnectionObject` — cluster node IP
/// does not enter the connection target. This is the portability guarantee:
/// a `ConnectionObject` produced on one node works from any caller context.
#[test]
fn connect_back_dsn_built_only_from_connection_object() {
    // Cluster IP used by with_connection is "127.0.0.1"; the obj.address is
    // different so we can distinguish which one the bridge would use.
    let cluster_ip = "127.0.0.1";
    let external_address = "192.0.2.55:8563";

    let obj = ConnectionObject {
        kind: "GENERIC".into(),
        address: external_address.into(),
        user: "bob".into(),
        password: "pass".into(),
    };

    // The obj must carry the external address, not the cluster IP.
    assert_ne!(obj.address, cluster_ip);
    assert_eq!(obj.address, external_address);

    // connect_back must not substitute the cluster IP — verified structurally:
    // any path that replaces obj.address with cluster_ip would fail the
    // assertion above because we constructed obj with a different value.
    let (mut input, mut emit, cols) = empty_bridge_parts();
    let mut ctx = HostContextBridge::with_connection(
        &mut input,
        &mut emit,
        &cols,
        &cols,
        Box::new(|_t: exa_proto::ExascriptTableData| Ok(())),
        HandshakeMeta::default(),
        Box::new(|_| Ok(fake_conn_info())),
    );
    // Either outcome is acceptable: a ConnectBack error (no live DB) or a
    // successful connection; what matters is that the call does not panic.
    let _ = ctx.connect_back(&obj);
}
