# Feature: db-roundtrip

Exercises the full Rust SLC against a live Exasol container across the supported version matrix — registering the slim SLC, uploading the UDF artifact to BucketFS, and invoking UDFs end-to-end to prove the wire protocol, dispatch, and SDK behave correctly against a real database. Connect-back end-to-end scenarios are specified separately in `integration/connect-back`.

## Background

The integration harness starts an `exasol/docker-db:<version>` container, registers the slim SLC, and uploads UDF `.so` artifacts to BucketFS with SSL verification disabled per project rules. The database version is selected at runtime by the `EXASOL_DB_SERIES` env var (`2025-1`, `2025-2`, `2026-1`); when unset it falls back to the series the binary was compiled with (default `2026-1`). A single `it-runner` binary, compiled once, drives every version in the matrix.

The pre-existing non-connect-back scenarios in this feature — `sanity_select_one`, `scalar_double_returns_42`, `set_filter_emits_positive_only`, `json_parse_extracts_name`, `udf_error_surfaces_prefix`, `single_call_default_output_columns_roundtrip`, and `single_call_unimplemented_returns_undefined` — are unaffected by the connect-back, `cluster_ip()`, and version-matrix changes in this plan. None of them exercise connect-back, so they are structurally safe; they MUST continue to pass unchanged.

## Scenarios

### Scenario: Harness starts Exasol and connects

* *GIVEN* a Docker daemon with the `exasol/docker-db:<version>` image available, where `<version>` is selected by the `EXASOL_VERSION` env var and defaults to `2026.1.0`
* *WHEN* the harness starts the container in privileged mode and waits for readiness
* *THEN* the database port `8563` and BucketFS port `2581` MUST be mapped to host ports
* *AND* an `exarrow-rs` connection to the mapped DB port MUST succeed and return a non-empty result for `SELECT 1`

### Scenario: Slim SLC is registered for the session

* *GIVEN* a running Exasol container and the locally built `slc-rs-slim:dev` image uploaded into BucketFS as the language container
* *WHEN* the harness runs `ALTER SESSION SET SCRIPT_LANGUAGES` with the `RUST=localzmq+protobuf://...#.../exaudf/exaudfclient` definition
* *THEN* the statement MUST succeed
* *AND* `RUST` MUST be usable as a script language alias in subsequent `CREATE SCRIPT` statements

### Scenario: UDF artifact is uploaded to BucketFS

* *GIVEN* a precompiled `libudf.so` built for `x86_64-unknown-linux-musl`
* *WHEN* the harness uploads it via HTTP PUT to `http://w:<write-password>@<host>:<bucketfs-port>/default/udfs/libscalar_double.so`
* *THEN* the upload MUST return a success status
* *AND* the file MUST be readable back from the same BucketFS path

### Scenario: Scalar UDF doubles a BIGINT input

* *GIVEN* the `scalar-double` UDF `.so` uploaded and a `CREATE OR REPLACE RUST SCALAR SCRIPT double_it(x BIGINT) RETURNS BIGINT` referencing its `%udf_object` path
* *WHEN* the harness runs `SELECT double_it(21)`
* *THEN* the query MUST return `42`

### Scenario: Set/EMITS UDF filters and emits rows

* *GIVEN* the `set-filter` UDF `.so` uploaded and a `CREATE OR REPLACE RUST SET SCRIPT filter_positive(x BIGINT) EMITS (x BIGINT)` referencing its `%udf_object` path
* *AND* an input table containing both positive and non-positive `BIGINT` values
* *WHEN* the harness runs the set UDF over the table and counts the emitted rows
* *THEN* the emitted row count MUST equal the number of positive input values
* *AND* every emitted value MUST be greater than zero

### Scenario: Third-party dependency is statically linked and usable

* *GIVEN* the `json-parse` UDF `.so` built with `serde_json` statically linked for `x86_64-unknown-linux-musl`
* *AND* a `CREATE OR REPLACE RUST SCALAR SCRIPT json_field(doc VARCHAR(2000)) RETURNS VARCHAR(2000)` referencing its `%udf_object` path
* *WHEN* the harness runs `json_field('{"name":"exa"}')`
* *THEN* the query MUST return `exa`
* *AND* the UDF MUST execute without any system-level `serde_json` library present in the slim image

### Scenario: UDF runtime error surfaces a prefixed message

* *GIVEN* a registered Rust UDF whose body returns an error for a given input
* *WHEN* the harness runs a query that triggers the error
* *THEN* the query MUST fail with an error message containing the `F-UDF-CL-RUST-` prefix

### Scenario: Single-call default-output-columns roundtrip returns a schema

* *GIVEN* a registered slim SLC session and a deployed UDF implementing `default_output_columns`
* *WHEN* the database issues the single-call `SC_FN_DEFAULT_OUTPUT_COLUMNS` against the UDF
* *THEN* the runtime MUST dispatch to the hook and reply `MT_RETURN` with the declared output columns
* *AND* the database MUST observe the returned column schema

### Scenario: Unimplemented single-call hook surfaces an undefined-call response

* *GIVEN* a registered slim SLC session and a deployed UDF that does not implement `generate_sql_for_export_spec`
* *WHEN* the database issues the single-call `SC_FN_GENERATE_SQL_FOR_EXPORT_SPEC`
* *THEN* the runtime MUST reply `MT_UNDEFINED_CALL`
* *AND* the database MUST treat the function as not provided rather than receiving a malformed result

### Scenario: Integration harness runs against the selected database version

* *GIVEN* the integration harness compiled once with the default `db-2026-1` Cargo feature and the `EXASOL_DB_SERIES` env var unset
* *WHEN* the harness runs without `EXASOL_DB_SERIES` set
* *THEN* the harness MUST resolve the database series to the compiled default (`2026-1`) and select the matching image tag and connect-back assertion severity
* *AND* WHEN the same compiled binary is run with `EXASOL_DB_SERIES` set to `2025-1`, `2025-2`, or `2026-1`, the harness MUST select the corresponding image tag and assertion severity at runtime without recompilation
* *AND* the harness MUST reject an unrecognised `EXASOL_DB_SERIES` value with a clear error rather than silently defaulting

