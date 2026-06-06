# Feature: db-roundtrip

Exercises the full Rust SLC against a live Exasol container — registering the slim SLC, uploading the UDF artifact to BucketFS, and invoking UDFs end-to-end to prove the wire protocol, dispatch, and SDK behave correctly against a real database.

## Background

The integration harness starts an `exasol/docker-db:2026.latest` container (identical image to `2026.1.0` at time of writing), registers the slim SLC, and uploads UDF `.so` artifacts to BucketFS with SSL verification disabled per project rules. Connect-back scenarios create a `CB_SELF` connection `TO '<routable-endpoint>:8563'` and register UDFs with a generic `%connection CB_SELF` directive. On `2026.latest` the connect-back round-trip is blocked by a server-side SIGABRT (signal 6) that terminates the invoking session when the core spawns the connect-back session process; those scenarios are therefore known-failing until a patched image ships. v2 adds roundtrips for connect-back (a UDF that queries the database from inside `run`), for DML connect-back (a UDF that creates a table and inserts rows, verified externally via `exapump`), and for the single-call path (`SC_FN_*` returning a schema or an undefined-call response).

Each scenario follows the same arc: upload `.so` to BucketFS over HTTP PUT (`http://w:<write-password>@<host>:<bucketfs-port>/<bucket>/<path>`), `ALTER SESSION SET SCRIPT_LANGUAGES` with the `RUST=localzmq+protobuf://...#.../exaudf/exaudfclient` definition, `CREATE OR REPLACE RUST {SCALAR|SET} SCRIPT` with the matching `%udf_object` path, run the query, assert. These tests actually start Exasol and run UDFs — they are not mocked. They are gated behind an `integration` feature/`#[ignore]`-style marker so they only run when Docker is available.

## Scenarios

### Scenario: Harness starts Exasol and connects

* *GIVEN* a Docker daemon with the `exasol/docker-db:2026.latest` image available
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

### Scenario: Connect-back UDF queries the database and emits the result

* *GIVEN* a registered slim SLC session and a deployed connect-back example UDF whose `run` calls `ctx.exa()?.query_arrow("SELECT 42")`
* *AND* a `CB_SELF` connection created `TO '<routable-endpoint>:8563'` reachable from the UDF sandbox, supplying the address/user/password the UDF connects with as an external client
* *WHEN* the UDF is invoked over the live Exasol container
* *THEN* on a supported database build the UDF MUST receive the query result as Arrow batches via a new external client session and emit the value `42`
* *AND* the connect-back session MUST be a new session and a new transaction, distinct from the invoking query's session
* *AND* on `exasol/docker-db:2026.latest` the harness MUST surface the documented server-side connect-back SIGABRT as a known failure with the connect-back diagnostic logs rather than masking it

### Scenario: Connect-back DML UDF inserts rows and data is visible externally

* *GIVEN* a registered slim SLC session and a deployed `connect-back-insert` UDF
* *AND* an input table containing three `BIGINT` rows `[10, 20, 30]`
* *AND* a `CB_SELF` connection created `TO '<routable-endpoint>:8563'` reachable from the UDF sandbox
* *WHEN* the UDF is invoked and its `run` creates `cb_result` via a new external client connect-back session and inserts each input value
* *THEN* on a supported database build `exapump` MUST be able to `SELECT val FROM cb_result ORDER BY val` against the same container and return exactly `[10, 20, 30]`, connecting with `validateservercertificate=0`
* *AND* on `exasol/docker-db:2026.latest` the harness MUST treat this scenario as a known failure for the same server-side connect-back SIGABRT as the connect-back query scenario

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

### Scenario: Connect-back UDF reaches a routable database endpoint without crashing the session

* *GIVEN* a registered slim SLC session and a deployed connect-back UDF registered with a generic `%connection CB_SELF` directive
* *AND* `CB_SELF` is created `TO '<routable-endpoint>:8563'` pointing at an address reachable from the UDF sandbox network namespace (the Docker host gateway plus the host-mapped DB port)
* *WHEN* the UDF is invoked and its `run` opens a connect-back connection and runs a query
* *THEN* on a patched database build the connect-back connection MUST succeed and the parent database session MUST remain alive throughout
* *AND* on `exasol/docker-db:2026.latest` the scenario MUST be treated as a known failure caused by a documented upstream defect — the parent session is terminated by signal 6 (core dumped) the moment the core spawns the connect-back session process, independent of the connect-back address or transport — and MUST NOT be attributed to the SLC until a patched `2026.x` image is published
