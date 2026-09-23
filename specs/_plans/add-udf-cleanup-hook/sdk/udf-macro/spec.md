# Feature: udf-macro

Defines the `#[exasol_udf]` proc-macro behaviour: compile-time code generation, entry-point wiring, panic safety, and type mapping. The macro generates the `cdylib` entry point and `ExaUdfVTable` from a function that takes `&mut dyn UdfContext`, deriving the output shape from the function's return type.

## Background

The `#[exasol_udf]` proc-macro turns an annotated function into deployable UDF entry points. The macro derives an SQL name (from the function identifier in `UPPER_SNAKE_CASE`, or verbatim via `name = "..."`) and namespaces every generated symbol with that name, exporting `__exa_udf_entry_<NAME>` instead of a single bare `__exa_udf_entry`. Same-name annotations still collide at link time; distinct-name annotations coexist in one crate. The macro inspects the annotated function's return type to choose the output shape and generates the matching `run` shim.

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: exasol_udf macro generates the entry point and vtable

* *GIVEN* a function annotated `#[exasol_udf]` (named `fn double_it`) that takes `&mut dyn UdfContext`
* *WHEN* the crate is compiled as a cdylib
* *THEN* the macro MUST derive the SQL entry-point name `DOUBLE_IT` by uppercasing the function identifier and generate an `extern "C"` `run` shim suffixed with it (`__exa_run_shim_DOUBLE_IT`), plus an `extern "C"` cleanup shim (`__exa_cleanup_shim_DOUBLE_IT`) only when the annotation carries a `cleanup(...)` section
* *AND* it MUST generate a `static` `__EXA_VTABLE_DOUBLE_IT` (marked `#[used]`) with `abi_version = EXA_UDF_ABI_VERSION` and the baked `sdk_fingerprint`
* *AND* it MUST generate `#[unsafe(no_mangle)] pub extern "C" fn __exa_udf_entry_DOUBLE_IT() -> *const ExaUdfVTable`
* *AND* it MUST NOT generate a bare `__exa_udf_entry` symbol (no suffix)
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: function name is translated to UPPER_SNAKE_CASE SQL name

* *GIVEN* a function annotated `#[exasol_udf]` named `fn double_it` with no `name = "..."` attribute
* *WHEN* the crate is compiled
* *THEN* the macro MUST translate the snake_case function identifier `double_it` to the UPPER_SNAKE_CASE SQL name `DOUBLE_IT` by ASCII-uppercasing each character (underscores preserved)
* *AND* every generated symbol (`__EXA_INPUT_SCHEMA_DOUBLE_IT`, `__EXA_OUTPUT_SCHEMA_DOUBLE_IT`, `__exa_write_c_string_DOUBLE_IT`, `__exa_run_shim_DOUBLE_IT`, `__EXA_VTABLE_DOUBLE_IT`, `__exa_udf_entry_DOUBLE_IT`, and `__exa_cleanup_shim_DOUBLE_IT` when a `cleanup(...)` section generates it) MUST carry that same `DOUBLE_IT` suffix
* *AND* the derived SQL name MUST match the bare object name the database sends as `script_name` for a `CREATE SCRIPT DOUBLE_IT`
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: name attribute overrides the SQL entry point name

* *GIVEN* a function annotated `#[exasol_udf(name = "MY_CUSTOM")]` named `fn double_it`
* *WHEN* the crate is compiled
* *THEN* the macro MUST use the verbatim attribute value `MY_CUSTOM` as the SQL entry-point name instead of deriving it from the function identifier
* *AND* the exported entry point MUST be `__exa_udf_entry_MY_CUSTOM`
* *AND* the `name = "..."` value MUST be combinable with the existing `input(...)`, `emits(...)`, `vs_adapter(...)`, `import_spec(...)`, `export_spec(...)`, and `cleanup(...)` sections in any order
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: Two exasol_udf annotations with distinct names produce independent entry points

* *GIVEN* a crate with two functions `fn double_it` and `fn triple_it`, each annotated `#[exasol_udf]`
* *WHEN* the crate is compiled as a cdylib
* *THEN* the build MUST succeed
* *AND* the artifact MUST export two distinct entry-point symbols `__exa_udf_entry_DOUBLE_IT` and `__exa_udf_entry_TRIPLE_IT`, each returning its own `*const ExaUdfVTable`
* *AND* each entry point MUST resolve to a vtable wired to its own `run` shim, its own cleanup shim when its annotation carries a `cleanup(...)` section, and its own annotated schema statics
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: Macro rejects an unknown annotation section by name

* *GIVEN* an `#[exasol_udf(...)]` annotation naming a section the macro does not define
* *WHEN* the crate is compiled
* *THEN* the macro MUST emit a compile error carrying the offending section's span
* *AND* the message MUST list every accepted section: `name`, `input`, `emits`, `vs_adapter`, `import_spec`, `export_spec`, and `cleanup`
<!-- /DELTA:CHANGED -->
