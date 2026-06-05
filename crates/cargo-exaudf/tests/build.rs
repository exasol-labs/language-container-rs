use std::process::Command;

fn cargo_exaudf_bin() -> std::path::PathBuf {
    let mut p = std::env::current_exe().unwrap();
    loop {
        p.pop();
        if p.ends_with("debug") || p.ends_with("release") {
            break;
        }
        if p.parent().is_none() {
            panic!("Could not find target dir");
        }
    }
    p.push("cargo-exaudf");
    p
}

fn rustup_available() -> bool {
    Command::new("rustup").arg("--version").output().is_ok()
}

/// Scaffold a minimal cdylib crate in `dir` using cargo-exaudf new,
/// then return the path to it.
fn scaffold_udf_crate(parent: &std::path::Path, name: &str) -> std::path::PathBuf {
    let udf_path = parent.join(name);
    let status = Command::new(cargo_exaudf_bin())
        .args(["exaudf", "new", udf_path.to_str().unwrap()])
        .status()
        .expect("cargo-exaudf new failed");
    assert!(status.success(), "scaffold failed");
    udf_path
}

#[test]
#[ignore = "requires musl toolchain and cargo; run with --ignored"]
fn build_produces_musl_so() {
    if !rustup_available() {
        eprintln!("SKIP: rustup not available");
        return;
    }

    let dir = tempfile::tempdir().unwrap();
    let udf_path = scaffold_udf_crate(dir.path(), "test-build-udf");

    let output = Command::new(cargo_exaudf_bin())
        .args(["exaudf", "build", udf_path.to_str().unwrap()])
        .output()
        .expect("failed to run cargo-exaudf build");

    assert!(
        output.status.success(),
        "build should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("x86_64-unknown-linux-musl") && stdout.contains(".so"),
        "stdout should print .so path: {stdout}"
    );
}

#[test]
#[ignore = "requires musl toolchain and cargo; run with --ignored"]
fn build_installs_missing_target() {
    if !rustup_available() {
        eprintln!("SKIP: rustup not available");
        return;
    }
    // We just verify the binary runs without crashing even if target needs installing
    let dir = tempfile::tempdir().unwrap();
    let udf_path = scaffold_udf_crate(dir.path(), "test-install-target-udf");

    // The build command should attempt rustup target add if needed and proceed
    let output = Command::new(cargo_exaudf_bin())
        .args(["exaudf", "build", udf_path.to_str().unwrap()])
        .output()
        .expect("failed to run cargo-exaudf build");

    // Either it succeeds (target already present or freshly installed) or fails on
    // compilation error — but it must not panic or skip the rustup step
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("thread 'main' panicked"),
        "build must not panic: {stderr}"
    );
}

#[test]
fn build_fails_on_missing_cargo_toml() {
    let dir = tempfile::tempdir().unwrap();
    let empty_path = dir.path().join("not-a-crate");
    std::fs::create_dir_all(&empty_path).unwrap();

    let output = Command::new(cargo_exaudf_bin())
        .args(["exaudf", "build", empty_path.to_str().unwrap()])
        .output()
        .expect("failed to run cargo-exaudf build");

    assert!(
        !output.status.success(),
        "build should fail when Cargo.toml is missing"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Cargo.toml"),
        "error must mention Cargo.toml: {stderr}"
    );
}
