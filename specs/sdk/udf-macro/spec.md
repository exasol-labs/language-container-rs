# Feature: udf-macro

Defines the `#[exasol_udf]` proc-macro behaviour: compile-time code generation, entry-point wiring, panic safety, and type mapping. The macro generates the `cdylib` entry point and `ExaUdfVTable` from a struct that implements `UdfRun`.

## Background

The `#[exasol_udf]` proc-macro is the author-facing annotation that turns a struct implementing `UdfRun` into a deployable `cdylib`. It generates `extern "C"` shims for `create`, `destroy`, and `run`, constructs a `static ExaUdfVTable` embedding the baked SDK fingerprint and ABI version, and exports `__exa_udf_entry`. Compile-time checks ensure unknown types and duplicate annotations are caught before producing a broken artifact.

## Scenarios

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

### Scenario: exasol_udf annotation with an unknown type fails to compile

* *GIVEN* an `#[exasol_udf]` annotation whose `input(...)` or `emits(...)` list names a Rust type the macro cannot map
* *WHEN* the crate is compiled
* *THEN* the macro MUST emit a compile error carrying the offending type's span
* *AND* the macro MUST map `i32`, `i64`, `f32`, `f64`, `bool`, `String`, and `&str`/`str` to their `ExaType` JSON names as before
* *AND* the macro MUST additionally map `Decimal`, `NaiveDate`, and `NaiveDateTime` to `Numeric`, `Date`, and `Timestamp` respectively so typed schema fields compile
