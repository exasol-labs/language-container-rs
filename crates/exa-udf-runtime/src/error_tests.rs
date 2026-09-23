use super::*;

#[test]
fn recorded_detail_is_appended_once() {
    let append = |text: &str, detail: Option<&str>| {
        RuntimeError::Udf(text.into())
            .with_recorded_detail(detail.map(Into::into))
            .to_string()
    };

    assert_eq!(
        append("gave up", Some("refused")),
        "UDF error: gave up: refused"
    );
    assert_eq!(
        append("gave up: refused", Some("refused")),
        "UDF error: gave up: refused"
    );
    assert_eq!(append("gave up", None), "UDF error: gave up");
}
