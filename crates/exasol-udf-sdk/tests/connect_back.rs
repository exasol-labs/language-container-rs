use exasol_udf_sdk::connect_back::{ConnectionObject, ExaConnection};
use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::error::UdfError;
use exasol_udf_sdk::value::Value;

// ---------------------------------------------------------------------------
// Mock connections — implement query_for_each (the required method), not
// query_arrow (which no longer exists on the trait surface).
// ---------------------------------------------------------------------------

struct MockConn {
    last_sql: Option<String>,
}

impl ExaConnection for MockConn {
    fn query_for_each(
        &mut self,
        sql: &str,
        f: &mut dyn FnMut(Vec<Value>) -> Result<(), UdfError>,
    ) -> Result<(), UdfError> {
        self.last_sql = Some(sql.to_string());
        for v in [10i64, 20, 30] {
            f(vec![Value::Int64(v)])?;
        }
        Ok(())
    }

    fn execute(&mut self, sql: &str) -> Result<u64, UdfError> {
        self.last_sql = Some(sql.to_string());
        Ok(3)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// `ExaConnection` is arrow-free: the trait surface contains only `Vec<Value>`
/// and it compiles without any feature gate.
#[test]
fn exaconnection_arrow_free_no_feature_gate() {
    let mut conn: Box<dyn ExaConnection> = Box::new(MockConn { last_sql: None });
    let rows = conn.query("SELECT v FROM t").unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0], vec![Value::Int64(10)]);
}

/// The mock implements `query_for_each` (the required method), not `query_arrow`
/// (which has been removed from the trait surface).
#[test]
fn mock_implements_query_for_each_not_query_arrow() {
    let mut conn = MockConn { last_sql: None };
    let mut collected = Vec::new();
    conn.query_for_each("SELECT v FROM t", &mut |row| {
        collected.push(row);
        Ok(())
    })
    .unwrap();
    assert_eq!(
        collected,
        vec![
            vec![Value::Int64(10)],
            vec![Value::Int64(20)],
            vec![Value::Int64(30)],
        ]
    );
    assert_eq!(conn.last_sql.as_deref(), Some("SELECT v FROM t"));
}

#[test]
fn execute_batch_default_returns_unimplemented() {
    let mut conn: Box<dyn ExaConnection> = Box::new(MockConn { last_sql: None });
    let rows = vec![vec![Value::Int64(1), Value::String("a".into())]];
    assert!(matches!(
        conn.execute_batch("INSERT INTO t VALUES (?, ?)", &rows),
        Err(UdfError::Unimplemented(_))
    ));
}

#[test]
fn transaction_methods_default_to_unimplemented() {
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
fn exa_connection_trait_query_and_execute() {
    // Object-safety: the trait must be usable behind a Box and Send.
    let mut conn: Box<dyn ExaConnection> = Box::new(MockConn { last_sql: None });

    let rows = conn.query("SELECT v FROM t").unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0], vec![Value::Int64(10)]);
    assert_eq!(rows[1], vec![Value::Int64(20)]);
    assert_eq!(rows[2], vec![Value::Int64(30)]);

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

/// cluster_ip/connection/connect_back are always declared (no feature gate).
/// Their defaults return Unimplemented — verifying they exist on every build.
#[test]
fn udfcontext_connect_back_methods_always_declared() {
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

// `emit_record_batch_ipc_present_without_emit_arrow` lives in tests/feature_gate.rs
// (the canonical location named in the plan's verification table).

#[test]
fn connect_back_accepts_caller_built_object() {
    let obj = ConnectionObject {
        kind: "JDBC".into(),
        address: "jdbc:postgresql://db.example.com/mydb".into(),
        user: "alice".into(),
        password: "hunter2".into(),
    };
    let mut ctx = MockCtx;
    let result = ctx.connect_back(&obj);
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// Multi-batch streaming via query_for_each
// ---------------------------------------------------------------------------

struct TwoBatchConn;

impl ExaConnection for TwoBatchConn {
    fn query_for_each(
        &mut self,
        _sql: &str,
        f: &mut dyn FnMut(Vec<Value>) -> Result<(), UdfError>,
    ) -> Result<(), UdfError> {
        // Simulates two batches: [1, 2] then [3, 4]
        for v in [1i64, 2] {
            f(vec![Value::Int64(v)])?;
        }
        for v in [3i64, 4] {
            f(vec![Value::Int64(v)])?;
        }
        Ok(())
    }

    fn execute(&mut self, _sql: &str) -> Result<u64, UdfError> {
        Ok(0)
    }
}

#[test]
fn query_for_each_streams_rows_in_order() {
    let mut conn = TwoBatchConn;

    let mut via_for_each: Vec<Vec<Value>> = Vec::new();
    conn.query_for_each("SELECT v FROM t", &mut |row| {
        via_for_each.push(row);
        Ok(())
    })
    .unwrap();

    assert_eq!(
        via_for_each,
        vec![
            vec![Value::Int64(1)],
            vec![Value::Int64(2)],
            vec![Value::Int64(3)],
            vec![Value::Int64(4)],
        ]
    );

    let via_query = conn.query("SELECT v FROM t").unwrap();
    assert_eq!(via_for_each, via_query);
}

struct ThreeRowConn;

impl ExaConnection for ThreeRowConn {
    fn query_for_each(
        &mut self,
        _sql: &str,
        f: &mut dyn FnMut(Vec<Value>) -> Result<(), UdfError>,
    ) -> Result<(), UdfError> {
        for v in [10i64, 20, 30] {
            f(vec![Value::Int64(v)])?;
        }
        Ok(())
    }

    fn execute(&mut self, _sql: &str) -> Result<u64, UdfError> {
        Ok(0)
    }
}

#[test]
fn query_for_each_stops_on_callback_error() {
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
    assert_eq!(call_count, 2);
}

// ---------------------------------------------------------------------------
// record_batch_to_rows helpers — only available with emit-arrow feature
// ---------------------------------------------------------------------------

#[cfg(feature = "emit-arrow")]
mod arrow_helpers {
    use arrow::array::Int64Array;
    use arrow::datatypes::{DataType, Field, Schema};
    use arrow::record_batch::RecordBatch;
    use exasol_udf_sdk::connect_back::{record_batch_to_rows, record_batches_to_rows};
    use std::sync::Arc;

    #[test]
    fn record_batch_to_rows_matches_multibatch() {
        let schema = Arc::new(Schema::new(vec![Field::new("v", DataType::Int64, false)]));

        let batch0 = RecordBatch::try_new(
            schema.clone(),
            vec![Arc::new(Int64Array::from(vec![1, 2, 3]))],
        )
        .unwrap();

        let batch1 =
            RecordBatch::try_new(schema.clone(), vec![Arc::new(Int64Array::from(vec![4, 5]))])
                .unwrap();

        let mut combined = record_batch_to_rows(&batch0).unwrap();
        combined.extend(record_batch_to_rows(&batch1).unwrap());

        let via_multi = record_batches_to_rows(&[batch0, batch1]).unwrap();

        assert_eq!(combined, via_multi);
    }
}
