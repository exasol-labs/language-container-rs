//! Connect-back unit tests using a mock [`ExaConnection`].
//!
//! These exercise the `HostContextBridge` connect-back wiring without a live
//! database: a mock connection counts the calls it receives and returns canned
//! Arrow batches, so the bridge's lazy-open and query plumbing can be verified
//! deterministically.

#![cfg(feature = "connect-back")]

use arrow::array::Int64Array;
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use exa_udf_runtime::{EmitBuffer, HostContextBridge, InputRowSet};
use exa_zmq_protocol::{ColumnMeta, ExaType};
use exasol_udf_sdk::connect_back::ExaConnection;
use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::error::UdfError;
use std::sync::Arc;

/// A mock connection that counts the calls it received and replays a single
/// canned batch, so tests can assert the result plumbing and connection reuse.
#[derive(Default)]
struct MockConnection {
    calls: usize,
}

fn one_int64_batch(value: i64) -> RecordBatch {
    let schema = Arc::new(Schema::new(vec![Field::new("n", DataType::Int64, false)]));
    RecordBatch::try_new(schema, vec![Arc::new(Int64Array::from(vec![value]))]).unwrap()
}

impl ExaConnection for MockConnection {
    fn query_arrow(&mut self, _sql: &str) -> Result<Vec<RecordBatch>, UdfError> {
        self.calls += 1;
        // Echo the running call count so reuse of the same connection (shared
        // mutable state) is observable through the trait object.
        Ok(vec![one_int64_batch(self.calls as i64)])
    }

    fn execute(&mut self, _sql: &str) -> Result<u64, UdfError> {
        self.calls += 1;
        Ok(self.calls as u64)
    }
}

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

#[test]
fn query_arrow_returns_record_batches() {
    let (mut input, mut emit, cols) = empty_bridge_parts();
    let mut bridge = HostContextBridge::with_connection(
        &mut input,
        &mut emit,
        &cols,
        Box::new(MockConnection::default()),
    );

    let batches = bridge.exa().unwrap().query_arrow("SELECT 42").unwrap();
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].num_rows(), 1);
    let col = batches[0]
        .column(0)
        .as_any()
        .downcast_ref::<Int64Array>()
        .unwrap();
    // First call on a fresh connection: the mock's call counter reads 1.
    assert_eq!(col.value(0), 1);
}

#[test]
fn exa_reuses_the_same_connection() {
    let (mut input, mut emit, cols) = empty_bridge_parts();
    let mut bridge = HostContextBridge::with_connection(
        &mut input,
        &mut emit,
        &cols,
        Box::new(MockConnection::default()),
    );

    // Two independent exa() calls must hand back the same underlying connection
    // so accumulated state (here: the recorded query log) is shared.
    bridge.exa().unwrap().query_arrow("SELECT 1").unwrap();
    bridge.exa().unwrap().query_arrow("SELECT 2").unwrap();

    // A third query proves the connection wasn't reopened between calls: the
    // execute path shares the same mock and its row count is stable.
    let rows = bridge.exa().unwrap().execute("DELETE FROM t").unwrap();
    assert_eq!(rows, 3);
}

#[test]
fn exa_without_connection_info_errors() {
    let (mut input, mut emit, cols) = empty_bridge_parts();
    // No connection injected and no handshake credentials: exa() must surface a
    // ConnectBack error rather than panic.
    let mut bridge = HostContextBridge::new(&mut input, &mut emit, &cols, None, None);

    match bridge.exa() {
        Err(UdfError::ConnectBack(_)) => {}
        Ok(_) => panic!("expected a ConnectBack error, got a connection"),
        Err(other) => panic!("expected a ConnectBack error, got {other:?}"),
    }
}
