# Feature: test-udfs

Provides the example UDF crates that the integration tests compile to fully-static musl `.so` artifacts and exercise against a real database: a scalar doubler, a set/EMITS filter, and a scalar JSON field extractor that statically links a third-party dependency.

## Background

Each test UDF is a `crate-type = ["cdylib"]` crate that depends only on `exasol-udf-sdk` (and, for the JSON case, `serde_json`). Each uses the bare `#[exasol_udf]` attribute on a struct implementing `UdfRun`. They are built for `x86_64-unknown-linux-musl` so all Rust dependencies are statically linked and the `.so` has no glibc or system-library requirement, matching what the slim image can load. These crates are workspace members under `test-udfs/` but are excluded from the default `cargo build --workspace` toolchain expectations because they target musl.

<!-- NEW -->

## Scenarios

### Scenario: scalar-double emits twice its input

* *GIVEN* the `scalar-double` crate with a `#[exasol_udf]` struct implementing `UdfRun`
* *WHEN* its `run` reads the first column as `i64` and emits `Value::Int64(x * 2)`
* *THEN* the crate MUST compile to a cdylib exporting `__exa_udf_entry`
* *AND* for input `21` the emitted value MUST be `42`

### Scenario: set-filter emits only positive rows

* *GIVEN* the `set-filter` crate with a `#[exasol_udf]` struct implementing `UdfRun`
* *WHEN* its `run` loops `ctx.next()` and emits each row whose first `i64` column is greater than zero
* *THEN* the crate MUST compile to a cdylib exporting `__exa_udf_entry`
* *AND* non-positive rows MUST NOT be emitted

### Scenario: json-parse extracts a field using serde_json

* *GIVEN* the `json-parse` crate depending on `serde_json` with a `#[exasol_udf]` struct
* *WHEN* its `run` reads the first column as a string, parses it with `serde_json`, and emits the `name` field as a string
* *THEN* the crate MUST compile to a cdylib for `x86_64-unknown-linux-musl` with `serde_json` statically linked
* *AND* for input `{"name":"exa"}` the emitted value MUST be `exa`

### Scenario: Test UDF .so builds for the musl target

* *GIVEN* any of the three test UDF crates
* *WHEN* it is built with `cargo build --release --target x86_64-unknown-linux-musl -p <crate>`
* *THEN* the build MUST produce `target/x86_64-unknown-linux-musl/release/lib<crate>.so`
* *AND* the artifact MUST export the `__exa_udf_entry` symbol

<!-- /NEW -->
