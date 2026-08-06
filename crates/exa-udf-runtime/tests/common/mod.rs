//! Shared helpers for the runtime's integration tests.

use std::path::PathBuf;

/// Absolute path to the fixture cdylib `lib<lib>.so`.
///
/// The fixtures are dev-dependencies of this crate, so Cargo builds them before
/// the tests run and rebuilds them when their sources change. Cargo does not
/// uplift a dependency's cdylib out of `deps/`, and test binaries live in that
/// same directory — so resolving the path relative to `current_exe()` finds the
/// fixture under any profile, target triple, or `CARGO_TARGET_DIR` (including
/// the `target/llvm-cov-target` redirect `cargo llvm-cov` installs).
pub fn fixture_so_path(lib: &str) -> PathBuf {
    let exe = std::env::current_exe().expect("current_exe");
    let so = exe
        .parent()
        .expect("test binary has a parent directory")
        .join(format!("lib{lib}.so"));
    assert!(
        so.exists(),
        "fixture cdylib not found: {so:?} — is `{lib}` a dev-dependency of exa-udf-runtime?"
    );
    so
}
