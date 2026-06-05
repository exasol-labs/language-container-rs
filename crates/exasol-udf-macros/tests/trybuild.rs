#[test]
fn macro_generates_entry() {
    let t = trybuild::TestCases::new();
    t.pass("tests/trybuild/single_entry.rs");
}

#[test]
fn duplicate_entry_link_error() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/trybuild/dup_entry.rs");
}
