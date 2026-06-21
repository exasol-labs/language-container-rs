# Feature: cargo-exaudf

Provides the `cargo exaudf` developer CLI that scaffolds a Rust UDF crate, builds it into a fully-static musl `.so`, and validates that a built `.so` is ABI-compatible with the current SDK — so authors never hand-manage the musl target triple or ABI constants.

## Background

The CLI validates and builds `.so` artifacts without a database, so it has no script name to resolve a single entry point. This delta changes `validate` and `build` to enumerate every exported `__exa_udf_entry_<NAME>` symbol (one per UDF in the crate) rather than resolving the single bare `__exa_udf_entry`. `validate` checks each discovered vtable's ABI/fingerprint and reports each UDF name; `build` verifies at least one named entry point exists. A `.so` with no named entry point — including legacy single-symbol artifacts — is rejected with a rebuild hint.

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: validate accepts a compatible .so

* *GIVEN* a `.so` built against the current `exasol-udf-sdk` exporting one or more `__exa_udf_entry_<NAME>` symbols
* *WHEN* the author runs `cargo exaudf validate <path.so>`
* *THEN* the CLI MUST dlopen the `.so` and discover every exported `__exa_udf_entry_<NAME>` symbol
* *AND* for each discovered entry point it MUST confirm the vtable `abi_version` equals `EXA_UDF_ABI_VERSION` and the `sdk_fingerprint` matches the current SDK
* *AND* it MUST report each discovered UDF name and exit zero reporting compatibility
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: validate rejects an ABI or fingerprint mismatch

* *GIVEN* a `.so` with a `__exa_udf_entry_<NAME>` symbol whose vtable `abi_version` or `sdk_fingerprint` differs from the current SDK
* *WHEN* the author runs `cargo exaudf validate <path.so>`
* *THEN* the CLI MUST exit non-zero
* *AND* it MUST report which UDF name and which of `abi_version` or `sdk_fingerprint` mismatched, showing expected and actual values
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: validate rejects a .so missing any entry symbol

* *GIVEN* a shared object that exports no `__exa_udf_entry_<NAME>` symbol (including a legacy `.so` that exports only the bare `__exa_udf_entry`)
* *WHEN* the author runs `cargo exaudf validate <path.so>`
* *THEN* the CLI MUST exit non-zero
* *AND* it MUST report that no `__exa_udf_entry_<NAME>` entry point could be found, with a hint to rebuild against sdk >= 0.14.0
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: build verifies the artifact exports at least one named entry point

* *GIVEN* an author crate annotated with `#[exasol_udf]`
* *WHEN* the author runs `cargo exaudf build`
* *THEN* after producing the musl `.so` the CLI MUST verify the artifact exports at least one `__exa_udf_entry_<NAME>` symbol resolving to a non-null vtable
* *AND* a build whose artifact exports no named entry point MUST fail with a clear error rather than producing a silently-unusable `.so`
<!-- /DELTA:CHANGED -->
