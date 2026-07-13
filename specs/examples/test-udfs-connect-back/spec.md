# Feature: test-udfs-connect-back

Provides the canonical example UDF crates that demonstrate connect-back (query and DML) and serve as fixtures for the integration tests.

## Background

Each example is a standalone cdylib crate depending only on `exasol-udf-sdk` with the `connect-back` feature and builds for the `x86_64-unknown-linux-musl` target. Connect-back logic is identical whether the registering script is `SCALAR` or `SET` — only the output path (return value versus `emit`) differs. Every fixture that an integration scenario `dlopen`s MUST be wired into the CI "Build UDF .so artifacts (release)" `-p` allowlist. Split out of `examples/test-udfs` to keep each example feature at or under ten scenarios.

## Scenarios

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

### Scenario: connect-back-scalar returns a value fetched over connect-back from a SCALAR script

* *GIVEN* the `connect-back-scalar` crate built against `exasol-udf-sdk` with the `connect-back` feature, registered as `RUST SCALAR SCRIPT connect_back_scalar() RETURNS BIGINT`
* *WHEN* its `run` calls `ctx.connection("CB_SELF")`, then `ctx.connect_back(&conn)`, then `conn.query("SELECT CAST(42 AS BIGINT)")`, and returns the first result cell as `Ok(Some(Value::Numeric(...)))`
* *THEN* the crate MUST compile to a cdylib for the `x86_64-unknown-linux-musl` target exporting `__exa_udf_entry_CONNECT_BACK_SCALAR` with the RETURNS output-shape marker
* *AND* `SELECT TO_CHAR(connect_back_scalar())` MUST return `42`
* *AND* the connect-back logic MUST be identical to the `connect-back-query` SET UDF; only the output path differs (this fixture returns its value, the SET fixture emits), because connect-back is not conditional on `SCALAR` versus `SET`
