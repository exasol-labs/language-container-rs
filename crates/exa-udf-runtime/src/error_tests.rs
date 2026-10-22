use super::*;

/// Scenario: A hook's own error text is surfaced unchanged
#[test]
fn recorded_detail_explains_only_a_textless_failure() {
    let detail = || Some("connection X does not exist".to_string());

    assert_eq!(
        RuntimeError::Unexplained("hook panicked".into())
            .with_recorded_detail(detail())
            .to_string(),
        "UDF error: hook panicked: connection X does not exist"
    );
    assert_eq!(
        RuntimeError::Unexplained("hook panicked".into())
            .with_recorded_detail(None)
            .to_string(),
        "UDF error: hook panicked"
    );
    assert_eq!(
        RuntimeError::Udf("no such source".into())
            .with_recorded_detail(detail())
            .to_string(),
        "UDF error: no such source"
    );
}

#[test]
fn protocol_error_has_one_prefix() {
    let e = RuntimeError::from(exa_zmq_protocol::ProtocolError::Protocol("boom".into()));
    assert_eq!(e.to_string(), "Protocol error: boom");
}
