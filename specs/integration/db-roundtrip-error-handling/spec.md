# Feature: db-roundtrip-error-handling

Exercises error and failure paths of the full Rust SLC against a live Exasol container — verifying that UDF runtime errors surface the correct prefix and that the full UDF-supplied error message text reaches the database without truncation. Happy-path and infrastructure scenarios are specified in `integration/db-roundtrip`.

## Background

The integration harness starts an `exasol/docker-db:<version>` container, registers the slim SLC, and uploads UDF `.so` artifacts to BucketFS with SSL verification disabled per project rules. The database version is selected at runtime by the `EXASOL_DB_SERIES` env var (`2025-1`, `2025-2`, `2026-1`); when unset it falls back to the series the binary was compiled with (default `2026-1`).

## Scenarios

### Scenario: UDF runtime error surfaces a prefixed message

* *GIVEN* a registered Rust UDF whose body returns an error for a given input
* *WHEN* the harness runs a query that triggers the error
* *THEN* the query MUST fail with an error message containing the `F-UDF-CL-RUST-` prefix

### Scenario: UDF error message content is surfaced without truncation

* *GIVEN* a registered Rust UDF whose body returns `Err(UdfError)` with a distinctive message for a given input, running against Exasol 2025.1.11 (`EXASOL_DB_SERIES=2025-1`)
* *WHEN* the harness runs a query that triggers the error
* *THEN* the query MUST fail with an error whose message contains the distinctive text returned by the UDF (e.g. `JSON parse error`), not only the `F-UDF-CL-RUST-` prefix and a generic error code
* *AND* the surfaced message MUST preserve the UDF-supplied text so the user can diagnose the failure
