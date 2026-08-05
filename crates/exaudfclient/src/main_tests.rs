use super::*;

fn args(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

#[test]
fn too_few_args_returns_exit_code_1() {
    let result = run(&args(&["exaudfclient", "tcp://localhost:1234"]), |_| {});
    let exit = result.unwrap_err();
    assert_eq!(exit.code, 1);
    assert!(exit.message.contains("F-UDF-CL-RUST-0003"));
}

#[test]
fn unsupported_lang_returns_exit_code_2() {
    let result = run(
        &args(&["exaudfclient", "tcp://localhost:1234", "lang=python"]),
        |_| {},
    );
    let exit = result.unwrap_err();
    assert_eq!(exit.code, 2);
    assert!(exit.message.contains("F-UDF-CL-RUST-0002"));
}

/// All parser-version resolution cases in one sequential test to avoid
/// env-var races between parallel test threads.
#[test]
fn resolve_parser_version_precedence() {
    // Ensure env var is absent for the non-env cases.
    // SAFETY: this test is intentionally single-threaded (see comment above);
    // no other threads read EXAUDF_PARSER_VERSION concurrently.
    unsafe { std::env::remove_var("EXAUDF_PARSER_VERSION") };

    // Default fallback.
    let v = resolve_parser_version(&args(&["exaudfclient", "tcp://x:1", "lang=rust"]));
    assert_eq!(v, "1");

    // Explicit arg overrides default.
    let v = resolve_parser_version(&args(&[
        "exaudfclient",
        "tcp://x:1",
        "lang=rust",
        "parser_version=7",
    ]));
    assert_eq!(v, "7");

    // Env var takes precedence over CLI arg.
    unsafe { std::env::set_var("EXAUDF_PARSER_VERSION", "42") };
    let v = resolve_parser_version(&args(&[
        "exaudfclient",
        "tcp://x:1",
        "lang=rust",
        "parser_version=7",
    ]));
    assert_eq!(v, "42");
    unsafe { std::env::remove_var("EXAUDF_PARSER_VERSION") };
}
