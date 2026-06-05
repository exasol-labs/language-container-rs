# Feature: udf-sdk

Provides the public Rust API that UDF authors depend on: the `Value` model, `ExaType`, the `UdfRun` and `UdfContext` traits, the stable C-ABI vtable, ABI constants, and the `#[exasol_udf]` attribute macro that wires an author's struct into a loadable cdylib.

## Background

This is the only crate (plus `arrow = "58"`) that a UDF author's `.so` links. The single ABI crossing is `extern "C" fn __exa_udf_entry() -> *const ExaUdfVTable`; rich trait objects never cross the boundary. The ABI is guarded by `EXA_UDF_ABI_VERSION = 1` and an `EXA_SDK_FINGERPRINT` of the form `"SDK_VERSION:RUSTC_HASH\0"`, baked via `build.rs`, so a `.so` built with a mismatched toolchain is rejected at load time rather than producing undefined behavior.

For v1 the SDK exposes scalar and set/EMITS execution. Connect-back (`ExaConnection`, `exa()`, `exa_named()`, `exa_connect()`), single-call hooks (`default_output_columns`, `generate_sql_for_import_spec`, `generate_sql_for_export_spec`), and typed schema annotations (`#[exasol_udf(input(...), emits(...))]`) are out of scope for v1; the macro accepts the bare `#[exasol_udf]` form only. The `connect-back` Cargo feature is declared but compiles to nothing in v1.

<!-- NEW -->

## Scenarios

### Scenario: Value and ExaType cover the v1 column types

* *GIVEN* the SDK `value` module
* *WHEN* a UDF reads or emits a column
* *THEN* `Value` MUST provide variants for `Null`, `Int32`, `Int64`, `Double`, `Numeric(i128, u8)`, `Bool`, `String`, `Date`, and `Timestamp`
* *AND* `ExaType` MUST provide the matching type descriptors, including `Numeric { precision, scale }` and `String { size }`

### Scenario: UdfContext exposes typed accessors and row iteration

* *GIVEN* the `UdfContext` trait
* *WHEN* a UDF inspects and reads its input
* *THEN* the trait MUST provide `next`, `reset`, and `emit`
* *AND* it MUST provide column introspection (`column_count`, `column_name`, `column_type`, `column_index`)
* *AND* it MUST provide typed accessors (`get_i64`, `get_f64`, `get_string`, `get_bool`, `get_decimal`, `get_date`, `get_timestamp`, `get_value`) where a SQL NULL maps to `None`

### Scenario: UdfRun default single-call hooks return Unimplemented

* *GIVEN* a struct that implements `UdfRun` providing only `run`
* *WHEN* a single-call hook (`default_output_columns`, `generate_sql_for_import_spec`, `generate_sql_for_export_spec`) is invoked
* *THEN* the default implementation MUST return `UdfError::Unimplemented`
* *AND* the trait MUST compile without the author providing those hooks

### Scenario: ABI constants and vtable layout are stable

* *GIVEN* the SDK `abi` module
* *WHEN* the vtable type is referenced by the host loader and the macro
* *THEN* `EXA_UDF_ABI_VERSION` MUST equal `1`
* *AND* `ExaUdfVTable` MUST be `#[repr(C)]` with fields `abi_version`, `sdk_fingerprint`, `create`, `destroy`, `run`, and optional `default_output_columns`, `generate_sql_import`, `generate_sql_export`

### Scenario: SDK fingerprint is baked at build time

* *GIVEN* the SDK `build.rs`
* *WHEN* the crate is compiled
* *THEN* it MUST set an `EXA_SDK_FINGERPRINT` value of the form `"SDK_VERSION:RUSTC_HASH\0"`
* *AND* the macro-generated vtable MUST embed that exact fingerprint string in its `sdk_fingerprint` field

### Scenario: exasol_udf macro generates the entry point and vtable

* *GIVEN* a struct annotated `#[exasol_udf]` that implements `UdfRun`
* *WHEN* the crate is compiled as a cdylib
* *THEN* the macro MUST generate `extern "C"` shims for `create`, `destroy`, and `run`
* *AND* it MUST generate a `static` `ExaUdfVTable` with `abi_version = EXA_UDF_ABI_VERSION` and the baked `sdk_fingerprint`
* *AND* it MUST generate `#[no_mangle] pub extern "C" fn __exa_udf_entry() -> *const ExaUdfVTable`

### Scenario: run shim catches panics and returns an error code

* *GIVEN* a UDF whose `run` panics
* *WHEN* the generated `run` shim invokes the user method
* *THEN* the shim MUST wrap the call in `catch_unwind`
* *AND* a caught panic MUST be converted to a non-zero error code rather than unwinding across the FFI boundary

### Scenario: Two exasol_udf annotations in one crate fail to link

* *GIVEN* a crate with two structs each annotated `#[exasol_udf]`
* *WHEN* the crate is compiled as a cdylib
* *THEN* the build MUST fail because of a duplicate `__exa_udf_entry` symbol
* *AND* the failure MUST occur at link time rather than producing a silently-wrong artifact

<!-- /NEW -->
