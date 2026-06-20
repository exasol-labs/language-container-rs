# Feature: udf-macro

Defines the `#[exasol_udf]` proc-macro behaviour: compile-time code generation, entry-point wiring, panic safety, and type mapping. The macro generates the `cdylib` entry point and `ExaUdfVTable` from a function that takes `&mut dyn UdfContext`.

## Background

The `#[exasol_udf]` proc-macro turns an annotated function into deployable UDF entry points. This delta makes one `cdylib` able to host many UDFs: the macro derives an SQL name (from the function identifier in `UPPER_SNAKE_CASE`, or verbatim via `name = "..."`) and namespaces every generated symbol with that name, exporting `__exa_udf_entry_<NAME>` instead of a single bare `__exa_udf_entry`. Same-name annotations still collide at link time; distinct-name annotations coexist.

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: exasol_udf macro generates the entry point and vtable

* *GIVEN* a function annotated `#[exasol_udf]` (named `fn double_it`) that takes `&mut dyn UdfContext`
* *WHEN* the crate is compiled as a cdylib
* *THEN* the macro MUST derive the SQL entry-point name `DOUBLE_IT` by uppercasing the function identifier and generate `extern "C"` `run`/`destroy` shims suffixed with it (`__exa_run_shim_DOUBLE_IT`, `__exa_destroy_shim_DOUBLE_IT`)
* *AND* it MUST generate a `static` `__EXA_VTABLE_DOUBLE_IT` (marked `#[used]`) with `abi_version = EXA_UDF_ABI_VERSION` and the baked `sdk_fingerprint`
* *AND* it MUST generate `#[unsafe(no_mangle)] pub extern "C" fn __exa_udf_entry_DOUBLE_IT() -> *const ExaUdfVTable`
* *AND* it MUST NOT generate a bare `__exa_udf_entry` symbol (no suffix)
<!-- /DELTA:CHANGED -->

### Scenario: run shim catches panics and returns an error code

* *GIVEN* a UDF whose `run` panics
* *WHEN* the generated `run` shim invokes the user method
* *THEN* the shim MUST wrap the call in `catch_unwind`
* *AND* a caught panic MUST be converted to a non-zero error code rather than unwinding across the FFI boundary

<!-- DELTA:NEW -->
### Scenario: function name is translated to UPPER_SNAKE_CASE SQL name

* *GIVEN* a function annotated `#[exasol_udf]` named `fn double_it` with no `name = "..."` attribute
* *WHEN* the crate is compiled
* *THEN* the macro MUST translate the snake_case function identifier `double_it` to the UPPER_SNAKE_CASE SQL name `DOUBLE_IT` by ASCII-uppercasing each character (underscores preserved)
* *AND* every generated symbol (`__EXA_INPUT_SCHEMA_DOUBLE_IT`, `__EXA_OUTPUT_SCHEMA_DOUBLE_IT`, `__exa_write_c_string_DOUBLE_IT`, `__exa_run_shim_DOUBLE_IT`, `__exa_destroy_shim_DOUBLE_IT`, `__EXA_VTABLE_DOUBLE_IT`, `__exa_udf_entry_DOUBLE_IT`) MUST carry that same `DOUBLE_IT` suffix
* *AND* the derived SQL name MUST match the bare object name the database sends as `script_name` for a `CREATE SCRIPT DOUBLE_IT`
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: name attribute overrides the SQL entry point name

* *GIVEN* a function annotated `#[exasol_udf(name = "MY_CUSTOM")]` named `fn double_it`
* *WHEN* the crate is compiled
* *THEN* the macro MUST use the verbatim attribute value `MY_CUSTOM` as the SQL entry-point name instead of deriving it from the function identifier
* *AND* the exported entry point MUST be `__exa_udf_entry_MY_CUSTOM`
* *AND* the `name = "..."` value MUST be combinable with the existing `input(...)`, `emits(...)`, and `vs_adapter(...)` sections in any order
<!-- /DELTA:NEW -->

<!-- DELTA:CHANGED -->
### Scenario: Two exasol_udf annotations with the same name fail to link

* *GIVEN* a crate with two functions whose `#[exasol_udf]` annotations resolve to the same SQL name (either two identical function identifiers in separate modules, or two `name = "DUP"` attributes)
* *WHEN* the crate is compiled as a cdylib
* *THEN* the build MUST fail because of a duplicate `__exa_udf_entry_DUP` symbol
* *AND* the failure MUST occur at link time rather than producing a silently-wrong artifact
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: Two exasol_udf annotations with distinct names produce independent entry points

* *GIVEN* a crate with two functions `fn double_it` and `fn triple_it`, each annotated `#[exasol_udf]`
* *WHEN* the crate is compiled as a cdylib
* *THEN* the build MUST succeed
* *AND* the artifact MUST export two distinct entry-point symbols `__exa_udf_entry_DOUBLE_IT` and `__exa_udf_entry_TRIPLE_IT`, each returning its own `*const ExaUdfVTable`
* *AND* each entry point MUST resolve to a vtable wired to its own `run`/`destroy` shims and its own annotated schema statics
<!-- /DELTA:NEW -->

### Scenario: exasol_udf annotation with an unknown type fails to compile

* *GIVEN* an `#[exasol_udf]` annotation whose `input(...)` or `emits(...)` list names a Rust type the macro cannot map
* *WHEN* the crate is compiled
* *THEN* the macro MUST emit a compile error carrying the offending type's span
* *AND* the macro MUST map `i32`, `i64`, `f32`, `f64`, `bool`, `String`, and `&str`/`str` to their `ExaType` JSON names as before
* *AND* the macro MUST additionally map `Decimal`, `NaiveDate`, and `NaiveDateTime` to `Numeric`, `Date`, and `Timestamp` respectively so typed schema fields compile
