# Feature: test-udfs

Provides the canonical example UDF crates that demonstrate each SDK capability and serve as fixtures for the integration tests.

## Background

Each example is a standalone cdylib crate depending on `exasol-udf-sdk` (plus the macro) and builds for the `x86_64-unknown-linux-musl` target. This delta proves the multi-entry-point capability end to end: annotated crates now export `__exa_udf_entry_<NAME>` symbols derived from their function identifiers, and `annotated-fixture` gains a second annotated function so a single `.so` exports two independent entry points.

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: annotated-double declares its schema via the typed annotation

* *GIVEN* an example UDF `fn annotated_double` annotated `#[exasol_udf(input(x: Decimal), emits(result: Decimal))]`
* *WHEN* the example is built for the musl target
* *THEN* the generated vtable MUST embed the input column `x: Numeric` and emit column `result: Numeric`
* *AND* the artifact MUST export the named entry point `__exa_udf_entry_ANNOTATED_DOUBLE` derived from the function identifier
* *AND* the example MUST double its input as in `scalar-double`
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: annotated-fixture exports two named entry points from one .so

* *GIVEN* the `annotated-fixture` crate declaring two annotated functions `fn annotated` and `fn annotated_double` in the same crate, each `#[exasol_udf]`
* *WHEN* the crate is built as a cdylib for the musl target
* *THEN* the single artifact MUST export both `__exa_udf_entry_ANNOTATED` and `__exa_udf_entry_ANNOTATED_DOUBLE`
* *AND* each entry point MUST return its own `*const ExaUdfVTable` with the matching annotated schema
* *AND* the build MUST succeed without a duplicate-symbol link error
<!-- /DELTA:NEW -->
