use super::*;
use exasol_udf_sdk::test_support::TestContext;

#[test]
fn current_user_meta_joins_five_fields_and_marks_absent_optionals() {
    let mut all_present = TestContext::scalar(Vec::new())
        .with_current_user("SYS")
        .with_scope_user("IT_VIEW_OWNER")
        .with_current_schema("IT_RUST_OTHER")
        .with_script_schema("IT_RUST")
        .with_script_name("CURRENT_USER_META");

    let rendered = current_user_meta(&mut all_present).unwrap();

    assert_eq!(
        rendered,
        Some("SYS|IT_VIEW_OWNER|IT_RUST_OTHER|IT_RUST|CURRENT_USER_META".to_string()),
        "the five fields must join in accessor order with no surrounding whitespace"
    );

    let mut optionals_absent = TestContext::scalar(Vec::new())
        .with_script_schema("IT_RUST")
        .with_script_name("CURRENT_USER_META");

    let rendered = current_user_meta(&mut optionals_absent).unwrap();

    assert_eq!(
        rendered,
        Some("<none>|<none>|<none>|IT_RUST|CURRENT_USER_META".to_string()),
        "an absent optional must render as the <none> marker, not as an empty field"
    );
}
