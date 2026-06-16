# Feature: db-roundtrip

Exercises the full Rust SLC against a live Exasol container across the supported version matrix — registering the slim SLC, uploading the UDF artifact to BucketFS, and invoking UDFs end-to-end to prove the wire protocol, dispatch, and SDK behave correctly against a real database. Connect-back end-to-end scenarios are specified separately in `integration/connect-back`.

## Background

The integration harness starts an `exasol/docker-db:<version>` container, registers the slim SLC, and uploads UDF `.so` artifacts to BucketFS with SSL verification disabled per project rules. The database version is selected at runtime by the `EXASOL_DB_SERIES` env var. The `json_parse` test UDF (registered as `json_field`) returns `Err(UdfError::User("JSON parse error: ..."))` on unparseable input, providing a UDF that returns an error with distinctive text. The existing `udf_error_surfaces_prefix` scenario verifies only that the `F-UDF-CL-RUST-` prefix appears; this delta adds a sibling scenario proving the UDF-supplied text itself reaches the SQL error. The text now travels out of the run shim through the vtable `run` out-pointer parameter — not through any connect-back channel — and dispatch folds it into the error-close message. This scenario MUST be verified against Exasol 2025.1.11 (`EXASOL_DB_SERIES=2025-1`).

## Scenarios

<!-- DELTA:NEW -->
### Scenario: UDF error message content is surfaced without truncation

* *GIVEN* a registered Rust UDF whose body returns `Err(UdfError)` with a distinctive message for a given input, running against Exasol 2025.1.11 (`EXASOL_DB_SERIES=2025-1`)
* *WHEN* the harness runs a query that triggers the error
* *THEN* the query MUST fail with an error whose message contains the distinctive text returned by the UDF (e.g. `JSON parse error`), not only the `F-UDF-CL-RUST-` prefix and a generic error code
* *AND* the surfaced message MUST preserve the UDF-supplied text so the user can diagnose the failure
<!-- /DELTA:NEW -->
