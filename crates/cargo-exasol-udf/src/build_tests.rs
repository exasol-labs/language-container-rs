use super::*;

#[test]
fn host_triple_maps_arch() {
    assert_eq!(host_triple("x86_64"), "x86_64-unknown-linux-musl");
    assert_eq!(host_triple("aarch64"), "aarch64-unknown-linux-musl");
}

#[test]
fn parse_build_args_empty_defaults_to_cwd_and_host_target() {
    let args: Vec<String> = vec![];
    let parsed = parse_build_args(&args).expect("empty args must parse");
    assert_eq!(parsed.path, ".");
    assert_eq!(parsed.target, host_triple(std::env::consts::ARCH));
}

#[test]
fn parse_build_args_positional_path_leaves_host_default_target() {
    let args: Vec<String> = vec!["my-udf".to_string()];
    let parsed = parse_build_args(&args).expect("positional path must parse");
    assert_eq!(parsed.path, "my-udf");
    assert_eq!(parsed.target, host_triple(std::env::consts::ARCH));
}

#[test]
fn parse_build_args_target_flag_overrides_default() {
    let args: Vec<String> = vec![
        "--target".to_string(),
        "aarch64-unknown-linux-musl".to_string(),
    ];
    let parsed = parse_build_args(&args).expect("--target with value must parse");
    assert_eq!(parsed.path, ".");
    assert_eq!(parsed.target, "aarch64-unknown-linux-musl");
}

#[test]
fn parse_build_args_positional_path_and_target_flag_both_set() {
    let args: Vec<String> = vec![
        "my-udf".to_string(),
        "--target".to_string(),
        "aarch64-unknown-linux-musl".to_string(),
    ];
    let parsed = parse_build_args(&args).expect("path + --target must parse");
    assert_eq!(parsed.path, "my-udf");
    assert_eq!(parsed.target, "aarch64-unknown-linux-musl");
}

#[test]
fn parse_build_args_dangling_target_flag_errors() {
    let args: Vec<String> = vec!["--target".to_string()];
    let err = parse_build_args(&args).expect_err("dangling --target must be rejected");
    assert!(
        err.contains("--target"),
        "error should name the offending flag: {err}"
    );
}
