#![cfg(feature = "connect-back")]

use arrow::array::Int64Array;
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use exasol_udf_sdk::connect_back::{ConnectBackOptions, ExaConnection};
use exasol_udf_sdk::error::UdfError;
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
fn exaconnection_trait_surface() {
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

#[test]
fn udfcontext_exposes_exa_methods() {
    // The ConnectBackOptions variants exist and carry the documented payloads.
    let _ = ConnectBackOptions::Default;
    let _ = ConnectBackOptions::Named("CONN_A".into());
    let explicit = ConnectBackOptions::Explicit {
        dsn: "192.0.2.1:8563".into(),
        user: "sys".into(),
        password: "exasol".into(),
    };
    match explicit {
        ConnectBackOptions::Explicit {
            dsn,
            user,
            password,
        } => {
            assert_eq!(dsn, "192.0.2.1:8563");
            assert_eq!(user, "sys");
            assert_eq!(password, "exasol");
        }
        _ => panic!("expected Explicit"),
    }
}
