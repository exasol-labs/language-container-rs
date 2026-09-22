# Feature: udf-macro

Defines the `#[exasol_udf]` proc-macro behaviour: compile-time code generation, entry-point wiring, panic safety, and type mapping. The macro generates the `cdylib` entry point and `ExaUdfVTable` from a function that takes `&mut dyn UdfContext`, deriving the output shape from the function's return type.

## Background

The `#[exasol_udf]` proc-macro turns an annotated function into deployable UDF entry points. The macro derives an SQL name (from the function identifier in `UPPER_SNAKE_CASE`, or verbatim via `name = "..."`) and namespaces every generated symbol with that name, exporting `__exa_udf_entry_<NAME>` instead of a single bare `__exa_udf_entry`. Same-name annotations still collide at link time; distinct-name annotations coexist in one crate. The macro inspects the annotated function's return type to choose the output shape and generates the matching `run` shim.

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: name attribute overrides the SQL entry point name

* *GIVEN* a function annotated `#[exasol_udf(name = "MY_CUSTOM")]` named `fn double_it`
* *WHEN* the crate is compiled
* *THEN* the macro MUST use the verbatim attribute value `MY_CUSTOM` as the SQL entry-point name instead of deriving it from the function identifier
* *AND* the exported entry point MUST be `__exa_udf_entry_MY_CUSTOM`
* *AND* the `name = "..."` value MUST be combinable with the existing `input(...)`, `emits(...)`, `vs_adapter(...)`, `import_spec(...)`, and `export_spec(...)` sections in any order
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: Macro rejects an unknown annotation section by name

* *GIVEN* an `#[exasol_udf(...)]` annotation naming a section the macro does not define
* *WHEN* the crate is compiled
* *THEN* the macro MUST emit a compile error carrying the offending section's span
* *AND* the message MUST list every accepted section: `name`, `input`, `emits`, `vs_adapter`, `import_spec`, and `export_spec`
<!-- /DELTA:NEW -->
