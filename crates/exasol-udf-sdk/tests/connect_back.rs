#![cfg(feature = "connect-back")]

use arrow::array::Int64Array;
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use exasol_udf_sdk::connect_back::{
    ConnectionObject, ExaConnection, record_batch_to_rows, record_batches_to_rows,
};
use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::error::UdfError;
use exasol_udf_sdk::value::Value;
use std::sync::Arc;

struct MockConn {
    last_sql: Option<String>,
}

impl ExaConnection for MockConn {
    fn query_arrow(&mut self, sql: &str) -> Result<Vec<RecordBatch>, UdfError> {
        self.last_sql = Some(sql.to_string());
        let schema = Arc::new(Schema::new(vec![Field::new("v", DataType::Int64, false)]));
        let batch =
            RecordBatch::try_new(schema, vec![Arc::new(Int64Array::from(vec![10, 20, 30]))])
                .map_err(|e| UdfError::ConnectBack(e.to_string()))?;
        Ok(vec![batch])
    }

    fn execute(&mut self, sql: &str) -> Result<u64, UdfError> {
        self.last_sql = Some(sql.to_string());
        Ok(3)
    }
}

#[test]
fn execute_batch_default_returns_unimplemented() {
    // MockConn only implements query_arrow + execute; execute_batch must
    // fall through to the trait default which returns Unimplemented.
    // This also verifies the method signature is exarrow-rs–free at the
    // SDK boundary (no Parameter type appears in the signature).
    let mut conn: Box<dyn ExaConnection> = Box::new(MockConn { last_sql: None });
    let rows = vec![vec![Value::Int64(1), Value::String("a".into())]];
    assert!(matches!(
        conn.execute_batch("INSERT INTO t VALUES (?, ?)", &rows),
        Err(UdfError::Unimplemented(_))
    ));
}

#[test]
fn transaction_methods_default_to_unimplemented() {
    // begin/commit/rollback exist on the trait surface so UDF code can call
    // them on a Box<dyn ExaConnection>. A connection that does not implement
    // them (the mock) inherits the defaults, which signal Unimplemented —
    // matching the cluster_ip/connection/connect_back convention.
    let mut conn: Box<dyn ExaConnection> = Box::new(MockConn { last_sql: None });
    assert!(matches!(conn.begin(), Err(UdfError::Unimplemented(_))));
    assert!(matches!(conn.commit(), Err(UdfError::Unimplemented(_))));
    assert!(matches!(conn.rollback(), Err(UdfError::Unimplemented(_))));
}

#[test]
fn connection_object_exposes_fields() {
    let obj = ConnectionObject {
        kind: "EXA".into(),
        address: "192.0.2.1:8563".into(),
        user: "sys".into(),
        password: "exasol".into(),
    };
    assert_eq!(obj.kind, "EXA");
    assert_eq!(obj.address, "192.0.2.1:8563");
    assert_eq!(obj.user, "sys");
    assert_eq!(obj.password, "exasol");
}

#[test]
fn exa_connection_trait_has_query_and_execute() {
    // Object-safety: the trait must be usable behind a Box, Send across moves.
    let mut conn: Box<dyn ExaConnection> = Box::new(MockConn { last_sql: None });

    let batches = conn.query_arrow("SELECT v FROM t").unwrap();
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].num_rows(), 3);

    let affected = conn.execute("INSERT INTO t VALUES (1)").unwrap();
    assert_eq!(affected, 3);

    fn assert_send<T: Send>(_: &T) {}
    assert_send(&conn);
}

struct MockCtx;

impl UdfContext for MockCtx {
    fn num_columns(&self) -> usize {
        0
    }
    fn get(&self, _col: usize) -> Result<&Value, UdfError> {
        Err(UdfError::Type("none".into()))
    }
    fn emit(&mut self, _values: &[Value]) -> Result<(), UdfError> {
        Ok(())
    }
    fn next(&mut self) -> Result<bool, UdfError> {
        Ok(false)
    }
}

#[test]
fn udfcontext_exposes_cluster_ip_connection_connect_back() {
    // The default implementations return Unimplemented — proving the methods
    // exist on the trait surface when the feature is enabled.
    let mut ctx = MockCtx;
    assert!(matches!(ctx.cluster_ip(), Err(UdfError::Unimplemented(_))));
    assert!(matches!(
        ctx.connection("CONN_A"),
        Err(UdfError::Unimplemented(_))
    ));
    let obj = ConnectionObject {
        kind: "EXA".into(),
        address: "192.0.2.1:8563".into(),
        user: "sys".into(),
        password: "secret".into(),
    };
    assert!(matches!(
        ctx.connect_back(&obj),
        Err(UdfError::Unimplemented(_))
    ));
}

#[test]
fn connect_back_accepts_caller_built_object() {
    // A caller can construct a ConnectionObject for a foreign target and pass it.
    let obj = ConnectionObject {
        kind: "JDBC".into(),
        address: "jdbc:postgresql://db.example.com/mydb".into(),
        user: "alice".into(),
        password: "hunter2".into(),
    };
    // The default impl returns Unimplemented; this just checks the type is
    // accepted without requiring a live connection.
    let mut ctx = MockCtx;
    let result = ctx.connect_back(&obj);
    assert!(result.is_err());
}

#[test]
fn record_batch_to_rows_matches_multibatch() {
    // Build two batches with a few rows each and verify that converting each
    // batch individually then concatenating produces the same result as the
    // multi-batch helper.
    let schema = Arc::new(Schema::new(vec![Field::new("v", DataType::Int64, false)]));

    let batch0 = RecordBatch::try_new(
        schema.clone(),
        vec![Arc::new(Int64Array::from(vec![1, 2, 3]))],
    )
    .unwrap();

    let batch1 =
        RecordBatch::try_new(schema.clone(), vec![Arc::new(Int64Array::from(vec![4, 5]))]).unwrap();

    let mut combined = record_batch_to_rows(&batch0).unwrap();
    combined.extend(record_batch_to_rows(&batch1).unwrap());

    let via_multi = record_batches_to_rows(&[batch0, batch1]).unwrap();

    assert_eq!(combined, via_multi);
}

/// Mock connection that returns two batches with known values.
struct TwoBatchConn;

impl ExaConnection for TwoBatchConn {
    fn query_arrow(&mut self, _sql: &str) -> Result<Vec<RecordBatch>, UdfError> {
        let schema = Arc::new(Schema::new(vec![Field::new("v", DataType::Int64, false)]));
        let batch0 =
            RecordBatch::try_new(schema.clone(), vec![Arc::new(Int64Array::from(vec![1, 2]))])
                .map_err(|e| UdfError::ConnectBack(e.to_string()))?;
        let batch1 = RecordBatch::try_new(schema, vec![Arc::new(Int64Array::from(vec![3, 4]))])
            .map_err(|e| UdfError::ConnectBack(e.to_string()))?;
        Ok(vec![batch0, batch1])
    }

    fn execute(&mut self, _sql: &str) -> Result<u64, UdfError> {
        Ok(0)
    }
}

#[test]
fn query_for_each_default_streams_rows() {
    // query_for_each must fire the callback in batch-then-row order: all rows
    // from batch 0 before any row from batch 1.
    let mut conn = TwoBatchConn;

    let mut via_for_each: Vec<Vec<Value>> = Vec::new();
    conn.query_for_each("SELECT v FROM t", &mut |row| {
        via_for_each.push(row);
        Ok(())
    })
    .unwrap();

    // Expected order: [1], [2], [3], [4]
    assert_eq!(
        via_for_each,
        vec![
            vec![Value::Int64(1)],
            vec![Value::Int64(2)],
            vec![Value::Int64(3)],
            vec![Value::Int64(4)],
        ]
    );

    // query (which now delegates to query_for_each) must produce the same rows.
    let via_query = conn.query("SELECT v FROM t").unwrap();
    assert_eq!(via_for_each, via_query);
}

/// Mock connection that returns one batch with three rows.
struct ThreeRowConn;

impl ExaConnection for ThreeRowConn {
    fn query_arrow(&mut self, _sql: &str) -> Result<Vec<RecordBatch>, UdfError> {
        let schema = Arc::new(Schema::new(vec![Field::new("v", DataType::Int64, false)]));
        let batch =
            RecordBatch::try_new(schema, vec![Arc::new(Int64Array::from(vec![10, 20, 30]))])
                .map_err(|e| UdfError::ConnectBack(e.to_string()))?;
        Ok(vec![batch])
    }

    fn execute(&mut self, _sql: &str) -> Result<u64, UdfError> {
        Ok(0)
    }
}

#[test]
fn query_for_each_stops_on_callback_error() {
    // The callback returns an error on the second row. query_for_each must
    // propagate that error and must NOT invoke the callback for the third row.
    let mut conn = ThreeRowConn;
    let mut call_count = 0usize;

    let result = conn.query_for_each("SELECT v FROM t", &mut |_row| {
        call_count += 1;
        if call_count == 2 {
            Err(UdfError::User("stop".into()))
        } else {
            Ok(())
        }
    });

    assert!(matches!(result, Err(UdfError::User(_))));
    // Callback was called for row 1 (Ok) and row 2 (Err), but NOT row 3.
    assert_eq!(call_count, 2);
}
