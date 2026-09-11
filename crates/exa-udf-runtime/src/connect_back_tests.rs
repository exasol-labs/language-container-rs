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
