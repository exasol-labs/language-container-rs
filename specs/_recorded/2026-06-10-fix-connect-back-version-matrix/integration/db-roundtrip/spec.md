# Feature: db-roundtrip

Exercises the full Rust SLC against a live Exasol container across the supported version matrix — registering the slim SLC, uploading the UDF artifact to BucketFS, and invoking UDFs end-to-end to prove the wire protocol, dispatch, and SDK behave correctly against a real database. Connect-back end-to-end scenarios are specified separately in `integration/connect-back`.

## Background

The integration harness starts an `exasol/docker-db:<version>` container, registers the slim SLC, and uploads UDF `.so` artifacts to BucketFS with SSL verification disabled per project rules. The database version is selected at runtime by the `EXASOL_DB_SERIES` env var (`2025-1`, `2025-2`, `2026-1`); when unset it falls back to the series the binary was compiled with (default `2026-1`). A single `it-runner` binary, compiled once, drives every version in the matrix.

The pre-existing non-connect-back scenarios in this feature — `sanity_select_one`, `scalar_double_returns_42`, `set_filter_emits_positive_only`, `json_parse_extracts_name`, `udf_error_surfaces_prefix`, `single_call_default_output_columns_roundtrip`, and `single_call_unimplemented_returns_undefined` — are unaffected by the connect-back, `cluster_ip()`, and version-matrix changes in this plan. None of them exercise connect-back, so they are structurally safe; they MUST continue to pass unchanged.

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Harness starts Exasol and connects

* *GIVEN* a Docker daemon with the `exasol/docker-db:<version>` image available, where `<version>` is selected by the `EXASOL_VERSION` env var and defaults to `2026.1.0`
* *WHEN* the harness starts the container in privileged mode and waits for readiness
* *THEN* the database port `8563` and BucketFS port `2581` MUST be mapped to host ports
* *AND* an `exarrow-rs` connection to the mapped DB port MUST succeed and return a non-empty result for `SELECT 1`
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: Integration harness runs against the selected database version

* *GIVEN* the integration harness compiled once with the default `db-2026-1` Cargo feature and the `EXASOL_DB_SERIES` env var unset
* *WHEN* the harness runs without `EXASOL_DB_SERIES` set
* *THEN* the harness MUST resolve the database series to the compiled default (`2026-1`) and select the matching image tag and connect-back assertion severity
* *AND* WHEN the same compiled binary is run with `EXASOL_DB_SERIES` set to `2025-1`, `2025-2`, or `2026-1`, the harness MUST select the corresponding image tag and assertion severity at runtime without recompilation
* *AND* the harness MUST reject an unrecognised `EXASOL_DB_SERIES` value with a clear error rather than silently defaulting
<!-- /DELTA:NEW -->
