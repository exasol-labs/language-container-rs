use std::env;

fn main() {
    let sdk_version = "0.1.1";
    let rustc_hash = {
        let rustc = env::var("RUSTC").unwrap_or_else(|_| "rustc".to_string());
        let output = std::process::Command::new(rustc)
            .arg("--version")
            .output()
            .expect("rustc not found");
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    };
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
    let fingerprint = format!("{}:{}", sdk_version, hash_part);
    println!("cargo:rustc-env=EXA_SDK_FINGERPRINT={}", fingerprint);
    println!("cargo:rerun-if-changed=build.rs");
}
