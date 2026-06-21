# Feature: test-udfs

Provides the canonical example UDF crates that demonstrate each SDK capability and serve as fixtures for the integration tests.

## Background

Each example is a standalone cdylib crate depending only on `exasol-udf-sdk` (plus `arrow` where needed) and builds for the `x86_64-unknown-linux-musl` target. Examples cover core UDF patterns (scalar, set, JSON, typed schema annotation), connect-back (query and DML), and multi-entry-point crates. Timestamp fixtures are in `examples/test-udfs-timestamps`.

## Scenarios

### Scenario: scalar-double emits twice its input

* *GIVEN* the `scalar-double` crate with a `#[exasol_udf]` struct implementing `UdfRun`
* *WHEN* its `run` reads the first column as `i64` and emits `Value::Int64(x * 2)`
* *THEN* the crate MUST compile to a cdylib exporting `__exa_udf_entry_SCALAR_DOUBLE`
* *AND* for input `21` the emitted value MUST be `42`

### Scenario: set-filter emits only positive rows

* *GIVEN* the `set-filter` crate with a `#[exasol_udf]` struct implementing `UdfRun`
* *WHEN* its `run` loops `ctx.next()` and emits each row whose first `i64` column is greater than zero
* *THEN* the crate MUST compile to a cdylib exporting `__exa_udf_entry_SET_FILTER`
* *AND* non-positive rows MUST NOT be emitted

### Scenario: json-parse extracts a field using serde_json

* *GIVEN* the `json-parse` crate depending on `serde_json` with a `#[exasol_udf]` struct
* *WHEN* its `run` reads the first column as a string, parses it with `serde_json`, and emits the `name` field as a string
* *THEN* the crate MUST compile to a cdylib for `x86_64-unknown-linux-musl` with `serde_json` statically linked
* *AND* for input `{"name":"exa"}` the emitted value MUST be `exa`

### Scenario: connect-back-query emits a value fetched over connect-back

* *GIVEN* an example UDF crate built against `exasol-udf-sdk` with the `connect-back` feature
* *WHEN* its `run` calls `ctx.exa()?.query_arrow("SELECT 42")` and emits the first cell
* *THEN* the example MUST compile as a cdylib for the musl target
* *AND* it MUST emit the integer fetched from the query

### Scenario: connect-back-insert creates a table and writes rows during run

* *GIVEN* an example UDF crate built against `exasol-udf-sdk` with the `connect-back` feature
* *WHEN* its `run` calls `ctx.exa()?.execute("CREATE TABLE IF NOT EXISTS cb_result (val BIGINT)")`, then for each input row calls `ctx.exa()?.execute(&format!("INSERT INTO cb_result VALUES ({})", value))`, and emits the row count
* *THEN* the example MUST compile as a cdylib for the musl target
* *AND* it MUST export the named entry point derived from the crate function name

### Scenario: annotated-double declares its schema via the typed annotation

* *GIVEN* an example UDF `fn annotated_double` annotated `#[exasol_udf(input(x: Decimal), emits(result: Decimal))]`
* *WHEN* the example is built for the musl target
* *THEN* the generated vtable MUST embed the input column `x: Numeric` and emit column `result: Numeric`
* *AND* the artifact MUST export the named entry point `__exa_udf_entry_ANNOTATED_DOUBLE` derived from the function identifier
* *AND* the example MUST double its input as in `scalar-double`

### Scenario: connect-back-scalar returns a value fetched over connect-back from a SCALAR script

* *GIVEN* the `connect-back-scalar` crate built against `exasol-udf-sdk` with the `connect-back` feature, registered as `RUST SCALAR SCRIPT connect_back_scalar() RETURNS BIGINT`
* *WHEN* its `run` calls `ctx.connection("CB_SELF")`, then `ctx.connect_back(&conn)`, then `conn.query("SELECT CAST(42 AS BIGINT)")`, and emits the first result cell as `Value::Numeric`
* *THEN* the crate MUST compile to a cdylib for the `x86_64-unknown-linux-musl` target exporting the named entry point `__exa_udf_entry_CONNECT_BACK_SCALAR`
* *AND* `SELECT TO_CHAR(connect_back_scalar())` MUST return `42`
* *AND* the implementation MUST be structurally identical to the `connect-back-query` SET UDF — connect-back logic is not conditional on `SCALAR` vs `SET` registration

### Scenario: annotated-fixture exports two named entry points from one .so

* *GIVEN* the `annotated-fixture` crate declaring two annotated functions `fn annotated` and `fn annotated_double` in the same crate, each `#[exasol_udf]`
* *WHEN* the crate is built as a cdylib for the musl target
* *THEN* the single artifact MUST export both `__exa_udf_entry_ANNOTATED` and `__exa_udf_entry_ANNOTATED_DOUBLE`
* *AND* each entry point MUST return its own `*const ExaUdfVTable` with the matching annotated schema
* *AND* the build MUST succeed without a duplicate-symbol link error
