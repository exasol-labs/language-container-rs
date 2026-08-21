// Wildcard import is the project's mandated `_tests.rs` sibling-module convention (CLAUDE.md).
#[allow(clippy::wildcard_imports)]
use super::*;

#[test]
fn write_usage_lists_every_subcommand_and_flag() {
    let mut buf = Vec::new();
    write_usage(&mut buf).unwrap();
    let text = String::from_utf8(buf).unwrap();

    assert!(text.contains("new <path>"));
    assert!(text.contains("build [<path>] [--target <triple>]"));
    assert!(text.contains("--target overrides the"));
    assert!(text.contains("validate <path>"));
    assert!(text.contains("--deny-unknown-deps"));
    assert!(text.contains("outside that surface"));
}
