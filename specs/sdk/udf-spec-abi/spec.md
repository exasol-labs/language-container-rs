# Feature: udf-spec-abi

Defines the `#[repr(C)]` ABI vtable slots for the `generate_sql_for_import_spec` and `generate_sql_for_export_spec` single-call hooks, and the `#[exasol_udf(import_spec(...), export_spec(...))]` macro wiring that fills them. The rest of the ABI vtable, fingerprint, and vtable stability rules are specified in `sdk/udf-abi`; the SDK trait-level hook contract these slots serve is specified in `sdk/udf-spec-hooks`.

## Background

The `generate_sql_for_import_spec` and `generate_sql_for_export_spec` vtable slots follow the same 3-argument ABI shape `virtual_schema_adapter_call` already established: `(ctx: *mut c_void, json_spec: *const c_char, result: *mut *mut c_char) -> i32`, with the same double-indirection `ctx` contract and the same `malloc`-backed C-string result convention. An author wires either slot with the `#[exasol_udf(import_spec(...))]` / `#[exasol_udf(export_spec(...))]` annotation; an omitted annotation leaves its slot `None`, so the runtime replies `MT_UNDEFINED_CALL` for that hook, preserving the behaviour of UDFs written before this change.

## Scenarios

### Scenario: import_spec and export_spec annotations wire the spec-generation slots

* *GIVEN* a function annotated `#[exasol_udf(import_spec(my_import_fn), export_spec(my_export_fn))]` where each target has the signature `fn(&mut dyn UdfContext, &str) -> Result<String, UdfError>`
* *WHEN* the crate is compiled as a cdylib
* *THEN* the macro MUST generate one `extern "C"` shim per annotation and wire it into the `generate_sql_for_import_spec` and `generate_sql_for_export_spec` vtable slots
* *AND* each shim MUST accept the 3-argument ABI `(ctx_ptr, json_spec, result)`, reconstruct `&mut dyn UdfContext` from `ctx_ptr` by double indirection, and call the annotated function
* *AND* on `Ok(s)` the shim MUST write `s` into a `malloc`-backed C string at `*result` and return `0`; on `Err(e)` write the error text and return `1`; on panic catch the unwind and return `2`
* *AND* interior NUL bytes in the returned SQL MUST be replaced with U+FFFD before writing to `*result`

### Scenario: An omitted spec annotation leaves its slot None

* *GIVEN* a function annotated `#[exasol_udf]` with no `import_spec` or `export_spec` clause
* *WHEN* the crate is compiled as a cdylib
* *THEN* both the `generate_sql_for_import_spec` and `generate_sql_for_export_spec` vtable slots MUST be `None`
* *AND* the runtime MUST reply `MT_UNDEFINED_CALL` when the DB invokes the matching `SC_FN_*` id, preserving the behaviour of UDFs written before this change

### Scenario: Spec-generation vtable slots take the context pointer, bumping the ABI version

* *GIVEN* the `generate_sql_for_import_spec` and `generate_sql_for_export_spec` slots of `ExaUdfVTable`
* *WHEN* the vtable is compiled under this change
* *THEN* both slot signatures MUST become `(ctx: *mut c_void, json_spec: *const c_char, result: *mut *mut c_char) -> i32`, identical to `virtual_schema_adapter_call`
* *AND* the `ctx` pointer MUST follow the same double-indirection contract the `run` and `virtual_schema_adapter_call` slots use, and the hook MUST NOT store it beyond the call
* *AND* `EXA_UDF_ABI_VERSION` MUST be bumped `9 → 10`, because both slot signatures and the `dyn UdfContext` method set changed, so a `.so` built against ABI 9 fails the loader's version check with a clear `AbiMismatch` error instead of being called with an extra argument
* *AND* the `#[repr(C)] ExaUdfVTable` field order MUST remain unchanged, so the version check itself stays readable across the boundary
* *AND* the `import` and `export` cargo features MUST change neither slot signature nor the `dyn UdfContext` method set, so both vtable layouts stay feature-independent and the raw `json_spec` string crosses the boundary in every build configuration
