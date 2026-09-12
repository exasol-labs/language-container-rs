use super::*;

#[test]
fn wide_has_24_columns_half_nullable_and_a_ddl_list() {
    assert_eq!(WIDE_COLUMNS.len(), 24);
    assert_eq!(WIDE_COLUMNS.iter().filter(|c| c.2).count(), 12);
    let ddl = wide_ddl();
    assert!(ddl.starts_with("k DECIMAL(18,0), i1 DECIMAL(18,0)"));
    assert!(ddl.ends_with("s200 VARCHAR(200)"));
    assert_eq!(ddl.matches(", ").count(), 23);
}

#[test]
fn varchar_size_parses_only_varchar() {
    assert_eq!(varchar_size("VARCHAR(200)"), Some(200));
    assert_eq!(varchar_size("DECIMAL(18,0)"), None);
    assert_eq!(varchar_size("VARCHAR(x)"), None);
}
