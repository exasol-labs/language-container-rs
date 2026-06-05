# Feature: udf-sdk

Defines the author-facing SDK — `UdfContext` and `UdfRun` traits, the `Value`/`ExaType` model, the `#[repr(C)]` ABI vtable, and the connect-back surface — that UDF crates depend on without linking the host runtime or exarrow-rs.

## Background

The SDK is the only crate a UDF author depends on (plus `arrow`). Its ABI constants and `#[repr(C)]` vtable are stable across the host loader and the proc macro. v2 adds the `virtual_schema_adapter_call` single-call hook, a feature-gated `connect-back` surface exposing the `ExaConnection` trait and `ConnectBackOptions` (with no exarrow-rs type in any public signature), and typed `#[exasol_udf(input(...), emits(...))]` annotation support that maps Rust types to `ExaType`.

## Scenarios

<!-- CHANGED -->
### Scenario: UdfRun default single-call hooks return Unimplemented

* *GIVEN* a struct that implements `UdfRun` providing only `run`
* *WHEN* a single-call hook (`default_output_columns`, `generate_sql_for_import_spec`, `generate_sql_for_export_spec`, `virtual_schema_adapter_call`) is invoked
* *THEN* the default implementation MUST return `UdfError::Unimplemented`
* *AND* the trait MUST compile without the author providing those hooks
<!-- /CHANGED -->

<!-- CHANGED -->
### Scenario: ABI constants and vtable layout are stable

* *GIVEN* the SDK `abi` module
* *WHEN* the vtable type is referenced by the host loader and the macro
* *THEN* `EXA_UDF_ABI_VERSION` MUST equal `1`
* *AND* `ExaUdfVTable` MUST be `#[repr(C)]` with fields `abi_version`, `sdk_fingerprint`, `create`, `destroy`, `run`, and optional `default_output_columns`, `generate_sql_import`, `generate_sql_export`, `virtual_schema_adapter_call`
<!-- /CHANGED -->

<!-- NEW -->
### Scenario: ExaConnection trait is defined behind the connect-back feature

* *GIVEN* the `exasol-udf-sdk` crate built with the `connect-back` feature enabled
* *WHEN* the `connect_back` module is referenced
* *THEN* it MUST expose an `ExaConnection` trait with `query_arrow`, `execute`, `import_arrow`, and `export_arrow` methods returning `Result<_, UdfError>`
* *AND* it MUST expose a `ConnectBackOptions` enum with `Default`, `Named(String)`, and `Explicit { host, user, password }` variants
* *AND* the `ExaConnection` trait MUST NOT reference any `exarrow-rs` type in its public signature
<!-- /NEW -->

<!-- NEW -->
### Scenario: UdfContext connect-back methods are absent without the feature

* *GIVEN* the `exasol-udf-sdk` crate built with the `connect-back` feature disabled
* *WHEN* the crate is compiled
* *THEN* the `UdfContext` methods `exa`, `exa_named`, and `exa_connect` MUST NOT be present
* *AND* the crate MUST NOT depend on `tokio` or `exarrow-rs`
<!-- /NEW -->

<!-- NEW -->
### Scenario: UdfContext exposes connect-back methods with the feature

* *GIVEN* the `exasol-udf-sdk` crate built with the `connect-back` feature enabled
* *WHEN* a UDF references the `UdfContext` trait
* *THEN* the trait MUST expose `exa(&self) -> Result<&dyn ExaConnection, UdfError>` for the lazy default connection
* *AND* it MUST expose `exa_named(&self, name: &str)` and `exa_connect(&self, opts: ConnectBackOptions)` each returning `Result<Box<dyn ExaConnection>, UdfError>`
<!-- /NEW -->

<!-- NEW -->
### Scenario: exasol_udf annotation generates schema metadata for matching types

* *GIVEN* a struct annotated `#[exasol_udf(input(x: i64, label: String), emits(result: i64))]`
* *WHEN* the crate is compiled as a cdylib
* *THEN* the macro MUST map each annotated Rust type to its `ExaType` (`i64` to `Int64`, `String` to `String`, `f64` to `Double`)
* *AND* it MUST embed the resulting input and emit column schema into the generated vtable for load-time validation
* *AND* the bare `#[exasol_udf]` form MUST continue to compile with no embedded schema
<!-- /NEW -->

<!-- NEW -->
### Scenario: exasol_udf annotation with an unknown type fails to compile

* *GIVEN* a struct annotated with an `input` or `emits` clause naming a type the macro cannot map to an `ExaType`
* *WHEN* the crate is compiled
* *THEN* the macro MUST emit a compile error naming the unsupported type
* *AND* the error MUST point at the offending annotation span
<!-- /NEW -->
