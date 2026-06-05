/// Integration-style CLI tests — invoke the binary and check exit codes / stderr.

#[test]
fn wrong_arg_count_rejected() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_exaudfclient"))
        .args(["tcp://localhost:1234"])
        .output()
        .expect("failed to run binary");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("F-UDF-CL-RUST-") || stderr.contains("Usage"),
        "unexpected stderr: {}",
        stderr
    );
}

#[test]
fn wrong_arg_count_exits_with_code_1() {
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_exaudfclient"))
        .args(["tcp://localhost:1234"])
        .status()
        .expect("failed to run binary");

    assert_eq!(status.code(), Some(1));
}

#[test]
fn unsupported_lang_rejected() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_exaudfclient"))
        .args(["tcp://localhost:1234", "lang=python"])
        .output()
        .expect("failed to run binary");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("F-UDF-CL-RUST-")
            || stderr.contains("unsupported")
            || stderr.contains("lang"),
        "unexpected stderr: {}",
        stderr
    );
}

#[test]
fn unsupported_lang_exits_with_code_2() {
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_exaudfclient"))
        .args(["tcp://localhost:1234", "lang=python"])
        .status()
        .expect("failed to run binary");

    assert_eq!(status.code(), Some(2));
}

#[test]
#[ignore = "requires live ZMQ endpoint"]
fn valid_invocation_delegates() {}

#[test]
#[ignore = "requires live ZMQ endpoint"]
fn runtime_failure_prefixed() {}
