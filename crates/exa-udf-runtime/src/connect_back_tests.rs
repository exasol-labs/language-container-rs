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
