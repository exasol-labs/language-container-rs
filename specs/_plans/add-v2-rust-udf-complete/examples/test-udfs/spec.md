# Feature: test-udfs

Provides the canonical example UDF crates that demonstrate each SDK capability and serve as fixtures for the integration tests.

## Background

Each example is a standalone cdylib crate depending only on `exasol-udf-sdk` (plus `arrow` where needed) and builds for the `x86_64-unknown-linux-musl` target. v2 adds a connect-back example that queries the database from inside `run`, a DML connect-back example that creates a table and inserts rows during `run`, and an annotated example that declares its schema via the typed `#[exasol_udf(input(...), emits(...))]` macro.

## Scenarios

<!-- NEW -->
### Scenario: connect-back-query emits a value fetched over connect-back

* *GIVEN* an example UDF crate built against `exasol-udf-sdk` with the `connect-back` feature
* *WHEN* its `run` calls `ctx.exa()?.query_arrow("SELECT 42")` and emits the first cell
* *THEN* the example MUST compile as a cdylib for the musl target
* *AND* it MUST emit the integer fetched from the query
<!-- /NEW -->

<!-- NEW -->
### Scenario: connect-back-insert creates a table and writes rows during run

* *GIVEN* an example UDF crate built against `exasol-udf-sdk` with the `connect-back` feature
* *WHEN* its `run` calls `ctx.exa()?.execute("CREATE TABLE IF NOT EXISTS cb_result (val BIGINT)")`, then for each input row calls `ctx.exa()?.execute(&format!("INSERT INTO cb_result VALUES ({})", value))`, and emits the row count
* *THEN* the example MUST compile as a cdylib for the musl target
* *AND* it MUST export the `__exa_udf_entry` symbol
<!-- /NEW -->

<!-- NEW -->
### Scenario: annotated-double declares its schema via the typed annotation

* *GIVEN* an example UDF annotated `#[exasol_udf(input(x: i64), emits(result: i64))]`
* *WHEN* the example is built for the musl target
* *THEN* the generated vtable MUST embed the input column `x: Int64` and emit column `result: Int64`
* *AND* the example MUST double its input as in `scalar-double`
<!-- /NEW -->
