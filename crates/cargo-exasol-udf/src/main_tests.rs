use super::*;

#[test]
fn write_usage_lists_every_subcommand_and_the_target_flag() {
    let mut buf = Vec::new();
    write_usage(&mut buf).unwrap();
    let text = String::from_utf8(buf).unwrap();

    assert!(text.contains("new <path>"));
    assert!(text.contains("build [<path>] [--target <triple>]"));
    assert!(text.contains("--target overrides the"));
    assert!(text.contains("validate <path>"));
}
