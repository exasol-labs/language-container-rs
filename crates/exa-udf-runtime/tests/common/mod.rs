//! Shared helpers for the runtime's integration tests.

use std::path::PathBuf;

/// Absolute path to the fixture cdylib for `lib`.
///
/// Fixtures are dependencies of this crate, and Cargo leaves a dependency's
/// cdylib in `deps/` beside the test binaries — so resolving against
/// `current_exe()` finds it under any profile or `CARGO_TARGET_DIR` (including
/// `cargo llvm-cov`'s `target/llvm-cov-target` redirect).
///
/// That layout is a Cargo implementation detail, not a guarantee, and the file
/// name assumes host cdylib naming. Scoped to cargo-driven host runs: a copied
/// test binary (as `it` does with `it-runner`) or a cross-compile would not
/// find the fixture.
pub fn fixture_cdylib_path(lib: &str) -> PathBuf {
    let exe = std::env::current_exe().expect("current_exe");
    let file = format!(
        "{}{lib}{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    );
    let so = exe
        .parent()
        .expect("test binary has a parent directory")
        .join(&file);
    assert!(
        so.exists(),
        "fixture cdylib not found: {so:?} — is `{lib}` a dependency of exa-udf-runtime?"
    );
    so
}
