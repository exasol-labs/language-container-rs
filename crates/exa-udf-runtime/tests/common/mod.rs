//! Shared helpers for the runtime's integration tests.

use std::path::PathBuf;

/// Absolute path to the fixture cdylib for `lib`.
///
/// The fixtures are dependencies of this crate, so Cargo builds them before the
/// tests run and rebuilds them when their sources change. Cargo does not uplift
/// a dependency's cdylib out of `deps/`, and test binaries live in that same
/// directory — so resolving the path relative to `current_exe()` finds the
/// fixture under any profile or `CARGO_TARGET_DIR` (including the
/// `target/llvm-cov-target` redirect `cargo llvm-cov` installs).
///
/// The file name uses the host's cdylib naming (`libfoo.so` on Linux,
/// `libfoo.dylib` on macOS) via [`std::env::consts`], which is what Cargo names
/// a dependency cdylib built for the host. Cross-compiling to a non-host triple
/// would need the *target*'s naming instead; the tests only build for the host.
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
