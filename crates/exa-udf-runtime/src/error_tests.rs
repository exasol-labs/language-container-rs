use super::*;

fn udf_error(text: &str) -> RuntimeError {
    RuntimeError::Udf(text.to_string())
}

#[test]
fn recorded_detail_is_appended_when_the_hook_text_lacks_it() {
    let error = udf_error("UDF cleanup returned error code 1: gave up")
        .with_recorded_detail(Some("Connect-back error: login refused".into()));

    assert_eq!(
        error.to_string(),
        "UDF error: UDF cleanup returned error code 1: gave up: Connect-back error: login refused"
    );
}

#[test]
fn recorded_detail_is_not_repeated_when_the_hook_already_reported_it() {
    let error = udf_error("UDF cleanup returned error code 1: Connect-back error: refused")
        .with_recorded_detail(Some("Connect-back error: refused".into()));

    assert_eq!(
        error.to_string(),
        "UDF error: UDF cleanup returned error code 1: Connect-back error: refused"
    );
}

#[test]
fn hook_error_without_recorded_detail_is_unchanged() {
    let error = udf_error("UDF cleanup returned error code 2").with_recorded_detail(None);

    assert_eq!(
        error.to_string(),
        "UDF error: UDF cleanup returned error code 2"
    );
}
