# Feature: udf-abi

Defines the `#[repr(C)]` ABI vtable, SDK fingerprint, vtable stability rules, and the `emit-arrow` feature boundary for the author-facing SDK.

## Background

The SDK ABI layer is the binary contract between a compiled UDF `.so` and the host runtime. The `#[repr(C)] ExaUdfVTable` carries an `abi_version`, an `sdk_fingerprint` (baked at build time from `SDK_VERSION:RUSTC_HASH`), a marker recording whether the UDF returns a value (RETURNS) or emits (EMITS), and function pointer slots for `run`, `destroy`, and optional single-call hooks. The `#[exasol_udf]` proc-macro generates the cdylib entry point and vtable. The host loader checks `abi_version` and `sdk_fingerprint` at load time; a mismatch is a clean `AbiMismatch` error rather than silent UB.

The `UdfContext` trait-object vtable is ordered by method declaration. Every `UdfContext` method must be declared unconditionally (no `#[cfg(feature = ...)]`) so the vtable layout is identical in all build configurations — a feature-mismatched `.so` must fail the version check, not misdispatch calls. A change to the signature of an existing method changes that slot's calling convention while leaving the layout intact, and requires the same version increment as an added or reordered slot. The `emit-arrow` feature gates only the optional `arrow` dependency and the `EmitBatch` extension trait; it never gates `UdfContext` method declarations.

## Scenarios

<!-- DELTA:NEW -->
### Scenario: import_spec and export_spec annotations wire the spec-generation slots

* *GIVEN* a function annotated `#[exasol_udf(import_spec(my_import_fn), export_spec(my_export_fn))]` where each target has the signature `fn(&mut dyn UdfContext, &str) -> Result<String, UdfError>`
* *WHEN* the crate is compiled as a cdylib
* *THEN* the macro MUST generate one `extern "C"` shim per annotation and wire it into the `generate_sql_for_import_spec` and `generate_sql_for_export_spec` vtable slots
* *AND* each shim MUST accept the 3-argument ABI `(ctx_ptr, json_spec, result)`, reconstruct `&mut dyn UdfContext` from `ctx_ptr` by double indirection, and call the annotated function
* *AND* on `Ok(s)` the shim MUST write `s` into a `malloc`-backed C string at `*result` and return `0`; on `Err(e)` write the error text and return `1`; on panic catch the unwind and return `2`
* *AND* interior NUL bytes in the returned SQL MUST be replaced with U+FFFD before writing to `*result`
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: An omitted spec annotation leaves its slot None

* *GIVEN* a function annotated `#[exasol_udf]` with no `import_spec` or `export_spec` clause
* *WHEN* the crate is compiled as a cdylib
* *THEN* both the `generate_sql_for_import_spec` and `generate_sql_for_export_spec` vtable slots MUST be `None`
* *AND* the runtime MUST reply `MT_UNDEFINED_CALL` when the DB invokes the matching `SC_FN_*` id, preserving the behaviour of UDFs written before this change
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: Spec-generation vtable slots take the context pointer, bumping the ABI version

* *GIVEN* the `generate_sql_for_import_spec` and `generate_sql_for_export_spec` slots of `ExaUdfVTable`
* *WHEN* the vtable is compiled under this change
* *THEN* both slot signatures MUST become `(ctx: *mut c_void, json_spec: *const c_char, result: *mut *mut c_char) -> i32`, identical to `virtual_schema_adapter_call`
* *AND* the `ctx` pointer MUST follow the same double-indirection contract the `run` and `virtual_schema_adapter_call` slots use, and the hook MUST NOT store it beyond the call
* *AND* `EXA_UDF_ABI_VERSION` MUST be bumped `9 → 10`, because both slot signatures and the `dyn UdfContext` method set changed, so a `.so` built against ABI 9 fails the loader's version check with a clear `AbiMismatch` error instead of being called with an extra argument
* *AND* the `#[repr(C)] ExaUdfVTable` field order MUST remain unchanged, so the version check itself stays readable across the boundary
* *AND* the `import` and `export` cargo features MUST change neither slot signature nor the `dyn UdfContext` method set, so both vtable layouts stay feature-independent and the raw `json_spec` string crosses the boundary in every build configuration
<!-- /DELTA:NEW -->
