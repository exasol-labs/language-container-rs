# Feature: cargo-exaudf

Provides the `cargo exaudf` developer CLI that scaffolds a Rust UDF crate, builds it into a fully-static musl `.so`, and validates that a built `.so` is ABI-compatible with the current SDK — so authors never hand-manage the musl target triple or ABI constants.

## Background

`cargo-exaudf` is a Cargo subcommand binary invoked as `cargo exaudf <subcommand>`. All builds target `x86_64-unknown-linux-musl`; the triple is hidden from the author. The CLI MUST NOT verify SSL certificates is not relevant here — it performs no network I/O. ABI compatibility is judged against the same `EXA_UDF_ABI_VERSION` and `EXA_SDK_FINGERPRINT` constants baked into `exasol-udf-sdk`.

## Scenarios

### Scenario: new scaffolds a buildable UDF crate

* *GIVEN* a target directory that does not yet contain a crate
* *WHEN* the author runs `cargo exaudf new my-udf`
* *THEN* the CLI MUST create a `Cargo.toml` with `crate-type = ["cdylib"]` and a dependency on `exasol-udf-sdk`
* *AND* it MUST create a `src/lib.rs` containing a `#[exasol_udf]` struct that implements `UdfRun`
* *AND* the generated crate MUST compile as written

### Scenario: new rejects an existing non-empty target

* *GIVEN* a target directory that already contains a `Cargo.toml`
* *WHEN* the author runs `cargo exaudf new` against that directory
* *THEN* the CLI MUST refuse to overwrite and exit with a non-zero status
* *AND* it MUST print a message naming the conflicting path

### Scenario: build produces a fully-static musl .so

* *GIVEN* a UDF crate produced by `cargo exaudf new`
* *WHEN* the author runs `cargo exaudf build`
* *THEN* the CLI MUST invoke `cargo build --release --target x86_64-unknown-linux-musl`
* *AND* it MUST print the path `target/x86_64-unknown-linux-musl/release/lib<crate>.so`

### Scenario: build installs the musl target when missing

* *GIVEN* a host where the `x86_64-unknown-linux-musl` target is not installed
* *WHEN* the author runs `cargo exaudf build`
* *THEN* the CLI MUST run `rustup target add x86_64-unknown-linux-musl` before building
* *AND* it MUST proceed with the build once the target is present

### Scenario: build emits a schema sidecar for annotated UDFs

* *GIVEN* a UDF crate whose struct is annotated `#[exasol_udf(input(...), emits(...))]`
* *WHEN* the author runs `cargo exaudf build`
* *THEN* the CLI MUST write a `<crate>.udf-meta.json` sidecar next to the `.so` describing the input and emit columns
* *AND* a bare `#[exasol_udf]` crate MUST build without producing a sidecar

### Scenario: validate accepts a compatible .so

* *GIVEN* a `.so` built against the current `exasol-udf-sdk`
* *WHEN* the author runs `cargo exaudf validate <path.so>`
* *THEN* the CLI MUST dlopen the `.so` and resolve the `__exa_udf_entry` symbol
* *AND* it MUST confirm the vtable `abi_version` equals `EXA_UDF_ABI_VERSION` and the `sdk_fingerprint` matches the current SDK
* *AND* it MUST exit zero reporting compatibility

### Scenario: validate rejects an ABI or fingerprint mismatch

* *GIVEN* a `.so` whose vtable `abi_version` or `sdk_fingerprint` differs from the current SDK
* *WHEN* the author runs `cargo exaudf validate <path.so>`
* *THEN* the CLI MUST exit non-zero
* *AND* it MUST report which of `abi_version` or `sdk_fingerprint` mismatched, showing expected and actual values

### Scenario: validate rejects a .so missing the entry symbol

* *GIVEN* a shared object that does not export `__exa_udf_entry`
* *WHEN* the author runs `cargo exaudf validate <path.so>`
* *THEN* the CLI MUST exit non-zero
* *AND* it MUST report that the `__exa_udf_entry` symbol could not be resolved
