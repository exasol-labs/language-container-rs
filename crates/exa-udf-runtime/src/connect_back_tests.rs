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

#[test]
fn dsn_preserves_special_chars_in_credentials() {
    let info = ConnInfo {
        kind: "EXASOL".into(),
        address: "10.0.0.1:8563".into(),
        user: "admin@corp".into(),
        password: "p@ss:w0rd?&=".into(),
    };
    let dsn = build_dsn(&info);
    assert_eq!(
        dsn,
        "exasol://admin@corp:p@ss:w0rd?&=@10.0.0.1:8563?validateservercertificate=0"
    );
}

#[test]
fn dsn_with_ipv6_address() {
    let info = ConnInfo {
        kind: "EXASOL".into(),
        address: "[::1]:8563".into(),
        user: "sys".into(),
        password: "exasol".into(),
    };
    let dsn = build_dsn(&info);
    assert!(
        dsn.contains("[::1]:8563"),
        "DSN must preserve IPv6 brackets; got: {dsn}"
    );
    assert!(dsn.ends_with("?validateservercertificate=0"));
}

#[test]
fn dsn_with_empty_credentials() {
    let info = ConnInfo {
        kind: "EXASOL".into(),
        address: "db:8563".into(),
        user: "".into(),
        password: "".into(),
    };
    let dsn = build_dsn(&info);
    assert_eq!(dsn, "exasol://:@db:8563?validateservercertificate=0");
}

#[test]
fn value_to_parameter_preserves_string_content() {
    let original = "hello, wörld! 🦀";
    let v = Value::String(original.to_string());
    match value_to_parameter(&v) {
        Ok(Parameter::String(s)) => assert_eq!(s, original),
        other => panic!("expected String parameter, got {other:?}"),
    }
}

#[test]
fn value_to_parameter_preserves_double_value() {
    let v = Value::Double(std::f64::consts::PI);
    match value_to_parameter(&v) {
        Ok(Parameter::Float(f)) => assert_eq!(f, std::f64::consts::PI),
        other => panic!("expected Float parameter, got {other:?}"),
    }
}

#[test]
fn value_to_parameter_int32_boundary_values() {
    match value_to_parameter(&Value::Int32(i32::MIN)) {
        Ok(Parameter::Integer(n)) => assert_eq!(n, i32::MIN as i64),
        other => panic!("expected Integer for i32::MIN, got {other:?}"),
    }
    match value_to_parameter(&Value::Int32(i32::MAX)) {
        Ok(Parameter::Integer(n)) => assert_eq!(n, i32::MAX as i64),
        other => panic!("expected Integer for i32::MAX, got {other:?}"),
    }
}

#[test]
fn value_to_parameter_int64_boundary_values() {
    match value_to_parameter(&Value::Int64(i64::MIN)) {
        Ok(Parameter::Integer(n)) => assert_eq!(n, i64::MIN),
        other => panic!("expected Integer for i64::MIN, got {other:?}"),
    }
    match value_to_parameter(&Value::Int64(i64::MAX)) {
        Ok(Parameter::Integer(n)) => assert_eq!(n, i64::MAX),
        other => panic!("expected Integer for i64::MAX, got {other:?}"),
    }
}

#[test]
fn connect_back_rt_returns_usable_runtime() {
    let rt = connect_back_rt();
    let result = rt.block_on(async { 42 });
    assert_eq!(result, 42);
}

#[test]
fn connect_back_rt_is_singleton() {
    let rt1 = connect_back_rt() as *const TokioRuntime;
    let rt2 = connect_back_rt() as *const TokioRuntime;
    assert_eq!(rt1, rt2, "connect_back_rt must return the same instance");
}

#[test]
fn ensure_rustls_provider_is_idempotent() {
    ensure_rustls_provider();
    ensure_rustls_provider();
}

// --- panic_message tests ---

#[test]
fn panic_message_extracts_static_str() {
    let payload: Box<dyn std::any::Any + Send> = Box::new("boom");
    assert_eq!(panic_message(&payload), "boom");
}

#[test]
fn panic_message_extracts_owned_string() {
    let payload: Box<dyn std::any::Any + Send> = Box::new(String::from("kaboom"));
    assert_eq!(panic_message(&payload), "kaboom");
}

#[test]
fn panic_message_falls_back_for_non_string_payload() {
    let payload: Box<dyn std::any::Any + Send> = Box::new(42i32);
    assert_eq!(panic_message(&payload), "unknown panic payload");
}

#[test]
fn panic_message_from_real_catch_unwind_str() {
    let result = std::panic::catch_unwind(|| panic!("test panic"));
    let payload = result.unwrap_err();
    assert_eq!(panic_message(&payload), "test panic");
}

#[test]
fn panic_message_from_real_catch_unwind_format() {
    let result = std::panic::catch_unwind(|| panic!("error: {}", 404));
    let payload = result.unwrap_err();
    assert_eq!(panic_message(&payload), "error: 404");
}

// --- ExaConnection trait behavior via mock ---

struct MockConnection {
    rows: Vec<Vec<Value>>,
    fail_at_row: Option<usize>,
}

impl ExaConnection for MockConnection {
    fn query_for_each(
        &mut self,
        _sql: &str,
        f: &mut dyn FnMut(Vec<Value>) -> Result<(), UdfError>,
    ) -> Result<(), UdfError> {
        for (i, row) in self.rows.clone().into_iter().enumerate() {
            if self.fail_at_row == Some(i) {
                return Err(UdfError::ConnectBack("simulated fetch error".into()));
            }
            f(row)?;
        }
        Ok(())
    }

    fn execute(&mut self, _sql: &str) -> Result<u64, UdfError> {
        Ok(self.rows.len() as u64)
    }
}

#[test]
fn query_collects_all_rows_from_query_for_each() {
    let mut conn = MockConnection {
        rows: vec![
            vec![Value::Int64(1)],
            vec![Value::Int64(2)],
            vec![Value::Int64(3)],
        ],
        fail_at_row: None,
    };
    let result = conn.query("SELECT 1").unwrap();
    assert_eq!(result.len(), 3);
    assert!(matches!(result[0][0], Value::Int64(1)));
    assert!(matches!(result[2][0], Value::Int64(3)));
}

#[test]
fn query_returns_empty_vec_for_no_rows() {
    let mut conn = MockConnection {
        rows: vec![],
        fail_at_row: None,
    };
    let result = conn.query("SELECT 1 WHERE FALSE").unwrap();
    assert!(result.is_empty());
}

#[test]
fn query_propagates_query_for_each_error() {
    let mut conn = MockConnection {
        rows: vec![vec![Value::Int64(1)], vec![Value::Int64(2)]],
        fail_at_row: Some(1),
    };
    let err = conn.query("SELECT 1").unwrap_err();
    assert!(matches!(err, UdfError::ConnectBack(_)));
}

#[test]
fn query_for_each_stops_on_callback_error() {
    let mut conn = MockConnection {
        rows: vec![
            vec![Value::Int64(1)],
            vec![Value::Int64(2)],
            vec![Value::Int64(3)],
        ],
        fail_at_row: None,
    };
    let mut seen = Vec::new();
    let err = conn
        .query_for_each("SELECT 1", &mut |row| {
            if let Value::Int64(n) = &row[0] {
                if *n >= 2 {
                    return Err(UdfError::User("stop".into()));
                }
                seen.push(*n);
            }
            Ok(())
        })
        .unwrap_err();
    assert!(matches!(err, UdfError::User(_)));
    assert_eq!(seen, vec![1], "callback must stop after the first error");
}

#[test]
fn trait_default_execute_batch_returns_unimplemented() {
    let mut conn = MockConnection {
        rows: vec![],
        fail_at_row: None,
    };
    let err = conn
        .execute_batch("INSERT INTO t VALUES (?)", &[vec![Value::Int64(1)]])
        .unwrap_err();
    assert!(matches!(err, UdfError::Unimplemented(_)));
}

#[test]
fn trait_default_begin_commit_rollback_return_unimplemented() {
    let mut conn = MockConnection {
        rows: vec![],
        fail_at_row: None,
    };
    assert!(matches!(conn.begin(), Err(UdfError::Unimplemented(_))));
    assert!(matches!(conn.commit(), Err(UdfError::Unimplemented(_))));
    assert!(matches!(conn.rollback(), Err(UdfError::Unimplemented(_))));
}
