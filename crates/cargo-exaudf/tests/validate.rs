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

#[test]
fn validate_rejects_missing_file() {
    let output = Command::new(cargo_exaudf_bin())
        .args(["exaudf", "validate", "/nonexistent/path/lib.so"])
        .output()
        .expect("failed to run cargo-exaudf");

    assert!(
        !output.status.success(),
        "validate should fail for missing file"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("not found")
            || stderr.contains("No such file")
            || stderr.contains("does not exist"),
        "error should mention missing file: {stderr}"
    );
}

#[test]
fn validate_rejects_missing_entry_symbol() {
    // Build a shared library that does NOT export __exa_udf_entry
    // We use a pre-compiled libc (which never has that symbol)
    // Alternatively, find any .so on the system that lacks the symbol
    // Use libpthread or similar system lib as a "no symbol" stand-in
    // Skip if we can't find a suitable .so
    let candidates = [
        "/usr/lib/x86_64-linux-gnu/libm.so.6",
        "/lib/x86_64-linux-gnu/libm.so.6",
        "/usr/lib/libm.so.6",
    ];
    let so_path = candidates.iter().find(|p| std::path::Path::new(p).exists());
    let so_path = match so_path {
        Some(p) => p,
        None => {
            eprintln!("SKIP: no system .so found for validate_rejects_missing_entry_symbol");
            return;
        }
    };

    let output = Command::new(cargo_exaudf_bin())
        .args(["exaudf", "validate", so_path])
        .output()
        .expect("failed to run cargo-exaudf");

    assert!(
        !output.status.success(),
        "validate should fail when __exa_udf_entry is missing"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("__exa_udf_entry") || stderr.contains("symbol") || stderr.contains("entry"),
        "error should mention missing symbol: {stderr}"
    );
}
