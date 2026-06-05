use std::env;

fn main() {
    let sdk_version = env!("CARGO_PKG_VERSION");
    // The rustc version string identifies the ABI-relevant toolchain. It must be
    // derived the same way for both the host runtime and the UDF `.so`, so it is
    // taken purely from `rustc --version` and never from build-time toggles like
    // `RUSTC_BOOTSTRAP` (which the UDF `cargo -Z build-std` invocation sets but
    // the client build does not — conflating them made the two fingerprints
    // disagree and every UDF load fail the fingerprint check).
    let rustc_hash = {
        let rustc = env::var("RUSTC").unwrap_or_else(|_| "rustc".to_string());
        let output = std::process::Command::new(rustc)
            .arg("--version")
            .output()
            .expect("rustc not found");
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    };
    // Sanitize: replace spaces with underscores, take first 64 chars
    let hash_part: String = rustc_hash
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '.' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .take(64)
        .collect();
    // The NUL terminator is appended in abi.rs via concat!, because env var
    // values pass through std::process::Command, which rejects interior NUL bytes.
    let fingerprint = format!("{}:{}", sdk_version, hash_part);
    println!("cargo:rustc-env=EXA_SDK_FINGERPRINT={}", fingerprint);
    println!("cargo:rerun-if-changed=build.rs");
}
