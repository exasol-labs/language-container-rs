use super::*;

#[test]
fn exatype_name_timestamp() {
    assert_eq!(
        exatype_name(&ExaType::Timestamp { precision: 3 }),
        "Timestamp"
    );
}

#[test]
fn exatype_name_string_and_char() {
    assert_eq!(exatype_name(&ExaType::String { size: 200 }), "String");
    assert_eq!(exatype_name(&ExaType::Char { size: 10 }), "String");
}
