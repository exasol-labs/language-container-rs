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
