# Feature: test-udfs

Provides the canonical example UDF crates that demonstrate each SDK capability and serve as fixtures for the integration tests.

## Background

Each example is a standalone cdylib crate depending only on `exasol-udf-sdk` (plus `arrow` where needed) and builds for the `x86_64-unknown-linux-musl` target. v2 adds a connect-back example that queries the database from inside `run`, a DML connect-back example that creates a table and inserts rows during `run`, and an annotated example that declares its schema via the typed `#[exasol_udf(input(...), emits(...))]` macro. This delta adds three timestamp fixtures used by the `db-roundtrip` suite to prove timestamp arithmetic, named-timezone resolution, and precision-preserving round-trips.

## Scenarios

### Scenario: scalar-double emits twice its input

* *GIVEN* the `scalar-double` crate with a `#[exasol_udf]` struct implementing `UdfRun`
* *WHEN* its `run` reads the first column as `i64` and emits `Value::Int64(x * 2)`
* *THEN* the crate MUST compile to a cdylib exporting `__exa_udf_entry`
* *AND* for input `21` the emitted value MUST be `42`

### Scenario: set-filter emits only positive rows

* *GIVEN* the `set-filter` crate with a `#[exasol_udf]` struct implementing `UdfRun`
* *WHEN* its `run` loops `ctx.next()` and emits each row whose first `i64` column is greater than zero
* *THEN* the crate MUST compile to a cdylib exporting `__exa_udf_entry`
* *AND* non-positive rows MUST NOT be emitted

### Scenario: json-parse extracts a field using serde_json

* *GIVEN* the `json-parse` crate depending on `serde_json` with a `#[exasol_udf]` struct
* *WHEN* its `run` reads the first column as a string, parses it with `serde_json`, and emits the `name` field as a string
* *THEN* the crate MUST compile to a cdylib for `x86_64-unknown-linux-musl` with `serde_json` statically linked
* *AND* for input `{"name":"exa"}` the emitted value MUST be `exa`

### Scenario: Test UDF .so builds for the musl target

* *GIVEN* any of the three test UDF crates
* *WHEN* it is built with `cargo build --release --target x86_64-unknown-linux-musl -p <crate>`
* *THEN* the build MUST produce `target/x86_64-unknown-linux-musl/release/lib<crate>.so`
* *AND* the artifact MUST export the `__exa_udf_entry` symbol

### Scenario: connect-back-query emits a value fetched over connect-back

* *GIVEN* an example UDF crate built against `exasol-udf-sdk` with the `connect-back` feature
* *WHEN* its `run` calls `ctx.exa()?.query_arrow("SELECT 42")` and emits the first cell
* *THEN* the example MUST compile as a cdylib for the musl target
* *AND* it MUST emit the integer fetched from the query

### Scenario: connect-back-insert creates a table and writes rows during run

* *GIVEN* an example UDF crate built against `exasol-udf-sdk` with the `connect-back` feature
* *WHEN* its `run` calls `ctx.exa()?.execute("CREATE TABLE IF NOT EXISTS cb_result (val BIGINT)")`, then for each input row calls `ctx.exa()?.execute(&format!("INSERT INTO cb_result VALUES ({})", value))`, and emits the row count
* *THEN* the example MUST compile as a cdylib for the musl target
* *AND* it MUST export the `__exa_udf_entry` symbol

### Scenario: annotated-double declares its schema via the typed annotation

* *GIVEN* an example UDF annotated `#[exasol_udf(input(x: i64), emits(result: i64))]`
* *WHEN* the example is built for the musl target
* *THEN* the generated vtable MUST embed the input column `x: Int64` and emit column `result: Int64`
* *AND* the example MUST double its input as in `scalar-double`

### Scenario: connect-back-scalar returns a value fetched over connect-back from a SCALAR script

* *GIVEN* the `connect-back-scalar` crate built against `exasol-udf-sdk` with the `connect-back` feature, registered as `RUST SCALAR SCRIPT connect_back_scalar() RETURNS BIGINT`
* *WHEN* its `run` calls `ctx.connection("CB_SELF")`, then `ctx.connect_back(&conn)`, then `conn.query("SELECT CAST(42 AS BIGINT)")`, and emits the first result cell as `Value::Numeric`
* *THEN* the crate MUST compile to a cdylib for the `x86_64-unknown-linux-musl` target exporting `__exa_udf_entry`
* *AND* `SELECT TO_CHAR(connect_back_scalar())` MUST return `42`
* *AND* the implementation MUST be structurally identical to the `connect-back-query` SET UDF — connect-back logic is not conditional on `SCALAR` vs `SET` registration

### Scenario: timestamp-add-second adds one second to a TIMESTAMP input

* *GIVEN* the `timestamp-add-second` crate with a `#[exasol_udf]` struct implementing `UdfRun`
* *WHEN* its `run` reads the first column as a `Value::Timestamp(NaiveDateTime)` and emits that timestamp plus one second as `Value::Timestamp`
* *THEN* the crate MUST compile to a cdylib for the `x86_64-unknown-linux-musl` target exporting `__exa_udf_entry`
* *AND* a `#[cfg(test)]` `TestCtx` test MUST assert that for input `2026-06-14 09:30:15.250000` the emitted value is `2026-06-14 09:30:16.250000`
* *AND* a NULL input MUST emit `Value::Null`

### Scenario: timestamp-now emits the local wall-clock time in the session timezone

* *GIVEN* the `timestamp-now` crate with a `#[exasol_udf]` struct implementing `UdfRun`
* *WHEN* its `run` reads the current local wall-clock time (resolving the `TZ` env via the zoneinfo database) and emits it as a `Value::Timestamp` whose naive value reflects that local time
* *THEN* the crate MUST compile to a cdylib for the `x86_64-unknown-linux-musl` target exporting `__exa_udf_entry`
* *AND* the implementation MUST obtain local time through a mechanism (`chrono::Local` or `time` with a tz feature) that consults the IANA zoneinfo database for a named `TZ`, so the emitted wall-clock value is correct when `tzdata` is present and would be UTC when it is absent
* *AND* the timezone resolution MUST be proven by the `integration/db-roundtrip` end-to-end scenario, not by a unit test, because zoneinfo resolution on a static-musl binary cannot be exercised in a host unit test

### Scenario: timestamp-passthrough reads and re-emits a TIMESTAMP unchanged

* *GIVEN* the `timestamp-passthrough` crate with a `#[exasol_udf]` struct implementing `UdfRun`
* *WHEN* its `run` reads the first column as `Value::Timestamp` and emits it unchanged
* *THEN* the crate MUST compile to a cdylib for the `x86_64-unknown-linux-musl` target exporting `__exa_udf_entry`
* *AND* a `#[cfg(test)]` `TestCtx` test MUST assert that a nanosecond-resolution timestamp (e.g. `2026-06-14 09:30:15.123456789`) passes through unchanged at the SDK `Value` level
* *AND* this UDF MUST serve as the precision-matrix fixture, so the declared output precision (set by the `RETURNS TIMESTAMP(p)` registration), not the UDF body, governs the wire-emitted fractional digits
