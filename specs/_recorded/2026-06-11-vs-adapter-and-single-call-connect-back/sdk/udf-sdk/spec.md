# Feature: udf-sdk

Defines the author-facing SDK — `UdfContext` and `UdfRun` traits, the `Value`/`ExaType` model, and the `#[repr(C)]` ABI vtable — that UDF crates depend on without linking the host runtime or exarrow-rs. The connect-back surface is specified separately in `sdk/connect-back`.

## Background

The SDK crate is a pure contract crate: it defines the ABI, trait interfaces, and value types. It does not link the host runtime or exarrow-rs. The `#[exasol_udf]` proc-macro generates the cdylib entry point and vtable from a struct that implements `UdfRun`. The SDK fingerprint, baked at build time from the SDK version and compiler hash, is embedded in the vtable for load-time compatibility checking by the host. ABI version 3 changes the `virtual_schema_adapter_call` vtable slot to a 3-argument signature that includes the host `UdfContext` pointer, enabling VS adapters to call `ctx.connection(...)` and `ctx.connect_back(...)` from inside single-call mode. This is a hard binary incompatibility with ABI v2 — the loader rejects v2 artifacts.

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: ABI constants and vtable layout are stable

* *GIVEN* the SDK `abi` module
* *WHEN* the vtable type is referenced by the host loader and the macro
* *THEN* `EXA_UDF_ABI_VERSION` MUST equal `3`
* *AND* `ExaUdfVTable` MUST be `#[repr(C)]` with fields `abi_version`, `sdk_fingerprint`, `run`, `destroy`, and optional `default_output_columns`, `virtual_schema_adapter_call`, `generate_sql_for_import_spec`, `generate_sql_for_export_spec`, `annotated_input_schema`, `annotated_output_schema`
* *AND* the `virtual_schema_adapter_call` slot MUST have the 3-argument signature `(ctx: *mut c_void, json_arg: *const c_char, result: *mut *mut c_char) -> i32`, where `ctx` is the same double-indirected `&mut dyn UdfContext` pointer the host passes to `run`
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: vs_adapter annotation wires the virtual_schema_adapter_call slot

* *GIVEN* a struct annotated `#[exasol_udf(vs_adapter(my_adapter_fn))]` where `my_adapter_fn` has the signature `fn(&mut dyn UdfContext, &str) -> Result<String, UdfError>`
* *WHEN* the crate is compiled as a cdylib
* *THEN* the macro MUST generate an `__exa_vs_adapter_shim` extern-C function and wire it into the `virtual_schema_adapter_call` vtable slot
* *AND* the shim MUST accept the 3-argument ABI `(ctx_ptr, json_arg, result)`, reconstruct `&mut dyn UdfContext` from `ctx_ptr` via double-indirection, and call `my_adapter_fn(ctx, json)`
* *AND* on `Ok(s)` the shim MUST write `s` into a `malloc`-backed C string at `*result` and return `0`; on `Err(e)` write the error and return `1`; on panic catch the unwind and return `2`
* *AND* interior NUL bytes in the response string MUST be replaced with U+FFFD before writing to `*result`

### Scenario: vs_adapter absent leaves slot None for backward compatibility

* *GIVEN* a struct annotated `#[exasol_udf]` with no `vs_adapter` clause
* *WHEN* the crate is compiled as a cdylib
* *THEN* the `virtual_schema_adapter_call` vtable slot MUST be `None`
* *AND* the runtime MUST reply `MT_UNDEFINED_CALL` when the DB invokes `SC_FN_VIRTUAL_SCHEMA_ADAPTER_CALL`, preserving backward compatibility with UDFs compiled before this change
<!-- /DELTA:NEW -->
