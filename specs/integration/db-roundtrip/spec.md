# Feature: db-roundtrip

Exercises the full Rust SLC against a live Exasol container across the supported version matrix — registering the slim SLC, uploading the UDF artifact to BucketFS, and invoking UDFs end-to-end to prove the wire protocol, dispatch, and SDK behave correctly against a real database. Error and failure scenarios are specified separately in `integration/db-roundtrip-error-handling`. Connect-back end-to-end scenarios are specified separately in `integration/connect-back`.

## Background

The integration harness starts an `exasol/docker-db:<version>` container, registers the slim SLC, and uploads UDF `.so` artifacts to BucketFS with SSL verification disabled per project rules. The database version is selected at runtime by the `EXASOL_DB_SERIES` env var (`2025-1`, `2025-2`, `2026-1`); when unset it falls back to the series the binary was compiled with (default `2026-1`). A single `it-runner` binary, compiled once, drives every version in the matrix. The DNS-gate scenario requires outbound network access from the runner so the external hostname resolves. Error and failure scenarios — including error-prefix verification and UDF error message content — are specified separately in `integration/db-roundtrip-error-handling`.

Before registering the Rust SLC, the harness runs an optional non-fatal Python3 built-in connect-back diagnostic. This diagnostic creates its own `CONNECTION`/`SCRIPT` objects (which are DB-global and persist regardless of session) and exercises connect-back via `connect_back_sql_address()`. Critically, the diagnostic runs on a DEDICATED throwaway connection, distinct from the shared connection that drives the asserted scenarios. If the diagnostic triggers a UDF VM crash (e.g. via a bad address), that crash is caught and logged as non-fatal; it MUST NOT affect the shared connection. The `CONNECTION` and `SCRIPT` objects created by the diagnostic are global and remain available for the asserted scenarios, even though the throwaway connection is closed immediately after.

The pre-existing non-connect-back scenarios in this feature — `sanity_select_one`, `scalar_double_returns_42`, `set_filter_emits_positive_only`, `json_parse_extracts_name`, `udf_error_surfaces_prefix`, `single_call_default_output_columns_roundtrip`, and `single_call_unimplemented_returns_undefined` — are unaffected by the connect-back, `cluster_ip()`, and version-matrix changes in this plan. None of them exercise connect-back, so they are structurally safe; they MUST continue to pass unchanged. This delta adds three timestamp scenarios — including the regression test for the `tzdata` packaging fix, which reports UTC on an image without `tzdata` and the correct Berlin offset with it.

## Scenarios

### Scenario: Harness starts Exasol and connects

* *GIVEN* a Docker daemon with the `exasol/docker-db:<version>` image available, where `<version>` is selected by the `EXASOL_VERSION` env var and defaults to `2026.1.0`
* *WHEN* the harness starts the container in privileged mode and waits for readiness
* *THEN* the database port `8563` and BucketFS port `2581` MUST be mapped to host ports
* *AND* an `exarrow-rs` connection to the mapped DB port MUST succeed and return a non-empty result for `SELECT 1`

### Scenario: Slim SLC is registered for the session

* *GIVEN* a running Exasol container and the locally built `lc-rs-slim:dev` image uploaded into BucketFS as the language container
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

### Scenario: Non-fatal Python3 connect-back diagnostic is session-isolated

* *GIVEN* the harness runs an optional Python3 built-in connect-back diagnostic before registering the Rust SLC
* *WHEN* the diagnostic creates its `CONNECTION`/`SCRIPT` objects and queries via connect-back on a DEDICATED throwaway connection (distinct from the shared connection that drives the asserted scenarios)
* *THEN* any failure or VM crash in the diagnostic MUST be caught and logged as non-fatal
* *AND* it MUST NOT affect the shared connection or any subsequent asserted scenario (e.g. `scalar_double_returns_42`), which run on the untouched shared connection
* *AND* the diagnostic's `CB_SELF_PY` connection MUST use `<container-eth0-ip>:8563` (never loopback), consistent with `connect_back_sql_address()`

### Scenario: DNS gate resolves an external hostname end-to-end

* *GIVEN* a running Exasol container with the slim Alpine SLC registered for the session
* *AND* the `resolv-udf` UDF uploaded and a SCALAR `RUST` script `resolv_udf` referencing its BucketFS `.so` path
* *WHEN* the harness runs `SELECT resolv_udf('www.exasol.com')` as part of the roundtrip suite
* *THEN* the query MUST return a single non-null VARCHAR value
* *AND* the returned string MUST parse as a valid `IpAddr`

### Scenario: Timestamp arithmetic round-trips through a SCALAR UDF

* *GIVEN* the `timestamp-add-second` UDF `.so` uploaded and a `CREATE OR REPLACE RUST SCALAR SCRIPT ts_add_second(t TIMESTAMP) RETURNS TIMESTAMP` referencing its `%udf_object` path
* *WHEN* the harness runs `SELECT ts_add_second(TIMESTAMP '2026-06-14 09:30:15.250000')` against the live `exasol/docker-db` container
* *THEN* the query MUST return a TIMESTAMP equal to the input plus exactly one second (`2026-06-14 09:30:16.250000`)
* *AND* the returned value MUST match the input's sub-second component, proving the timestamp survives the decode/emit round-trip rather than being zeroed or truncated

### Scenario: UDF local time agrees with the session timezone and is not UTC

* *GIVEN* the `timestamp-now` UDF `.so` uploaded and a `CREATE OR REPLACE RUST SCALAR SCRIPT udf_now() RETURNS TIMESTAMP` referencing its `%udf_object` path
* *AND* the session timezone set to a named non-UTC zone via `ALTER SESSION SET TIME_ZONE='Europe/Berlin'`
* *WHEN* the harness runs `SELECT udf_now()` and `SELECT CURRENT_TIMESTAMP` on the same session
* *THEN* the UDF wall-clock value MUST agree with the DB `CURRENT_TIMESTAMP` within a bounded skew tolerance of a few seconds (covering execution latency)
* *AND* the UDF wall-clock value MUST NOT equal the UTC wall-clock time for the same instant — its offset MUST be the Berlin offset (`+01:00` or `+02:00` depending on DST), proving the named zone resolved from the bundled `tzdata`
* *AND* this scenario is the regression test for the `tzdata` packaging fix: it MUST fail (UDF reports UTC) against an Alpine image built without `tzdata` and MUST pass against an image with `tzdata`

### Scenario: TIMESTAMP fractional precision round-trips through a UDF at the microsecond input cap for the 0/3/6/9 matrix

* *GIVEN* the `timestamp-passthrough` UDF `.so` uploaded
* *AND* for each precision `p` in {0, 3, 6, 9} a `CREATE OR REPLACE RUST SCALAR SCRIPT ts_pass_p(t TIMESTAMP(p)) RETURNS TIMESTAMP(p)` referencing its `%udf_object` path (verifying the exact `TIMESTAMP(p)` registration syntax the DB accepts)
* *WHEN* the harness runs each `ts_pass_p` over a literal carrying `p` significant fractional digits (e.g. `123456789` truncated to `p`)
* *THEN* each returned value MUST equal the input literal truncated to `min(p, 6)` fractional digits and widened back to `TIMESTAMP(p)` — because the DB delivers every UDF *input* column at microsecond precision (`SWIGTableData::getTimestamp` formats `...FF6`, verified in the engine source and empirically across Rust, Python, and Java), so the realistic round-trip ceiling through any UDF is microseconds
* *AND* for `p` in {0, 3, 6} the round-trip MUST be exactly lossless — equal to what the DB stores for the same `TIMESTAMP(p)` literal — since the literal carries no precision beyond microseconds to lose, and `p = 0` MUST NOT be padded with a spurious `.000000` (the engine truncates the emitted fraction away)
* *AND* for `p = 9` the UDF MUST receive `2026-06-14 09:30:15.123456000` (digits 7–9 dropped on the input wire, not by the SLC), so the round-trip MUST return `.123456000`, NOT the stored literal `.123456789`; the `%.9f` emit format cannot recover precision the input wire already discarded (the emit format's nanosecond capability benefits only UDF-*generated* sub-microsecond values, proven separately by the `timestamp_emit_nanosecond_roundtrip` unit test)
