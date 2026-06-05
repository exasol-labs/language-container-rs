# Feature: db-roundtrip

Exercises the full Rust SLC against a live Exasol container — registering the slim SLC, uploading the UDF artifact to BucketFS, and invoking UDFs end-to-end to prove the wire protocol, dispatch, and SDK behave correctly against a real database.

## Background

The integration harness starts an Exasol `2026.latest` container, registers the slim SLC for the session, and uploads UDF `.so` artifacts to BucketFS (SSL verification disabled per project rules). v2 adds roundtrips for connect-back (a UDF that queries the database from inside `run`), for DML connect-back (a UDF that creates a table and inserts rows, verified externally via `exapump`), and for the single-call path (`SC_FN_*` returning a schema or an undefined-call response).

## Scenarios

<!-- NEW -->
### Scenario: Connect-back UDF queries the database and emits the result

* *GIVEN* a registered slim SLC session and a deployed connect-back example UDF whose `run` calls `ctx.exa()?.query_arrow("SELECT 42")`
* *WHEN* the UDF is invoked over the live Exasol container
* *THEN* the UDF MUST receive the query result as Arrow batches via connect-back
* *AND* it MUST emit the value `42`
* *AND* the query MUST execute against the same database session credentials delivered in the handshake
<!-- /NEW -->

<!-- NEW -->
### Scenario: Connect-back DML UDF inserts rows and data is visible externally

* *GIVEN* a registered slim SLC session and a deployed `connect-back-insert` UDF
* *AND* an input table containing three `BIGINT` rows `[10, 20, 30]`
* *WHEN* the UDF is invoked and its `run` creates `cb_result` via connect-back and inserts each input value
* *THEN* the UDF MUST emit the row count `3`
* *AND* after the query completes, `exapump` MUST be able to `SELECT val FROM cb_result ORDER BY val` against the same container and return exactly `[10, 20, 30]`
* *AND* `exapump` MUST connect with `validateservercertificate=0`
<!-- /NEW -->

<!-- NEW -->
### Scenario: Single-call default-output-columns roundtrip returns a schema

* *GIVEN* a registered slim SLC session and a deployed UDF implementing `default_output_columns`
* *WHEN* the database issues the single-call `SC_FN_DEFAULT_OUTPUT_COLUMNS` against the UDF
* *THEN* the runtime MUST dispatch to the hook and reply `MT_RETURN` with the declared output columns
* *AND* the database MUST observe the returned column schema
<!-- /NEW -->

<!-- NEW -->
### Scenario: Unimplemented single-call hook surfaces an undefined-call response

* *GIVEN* a registered slim SLC session and a deployed UDF that does not implement `generate_sql_for_export_spec`
* *WHEN* the database issues the single-call `SC_FN_GENERATE_SQL_FOR_EXPORT_SPEC`
* *THEN* the runtime MUST reply `MT_UNDEFINED_CALL`
* *AND* the database MUST treat the function as not provided rather than receiving a malformed result
<!-- /NEW -->

<!-- NEW -->
### Scenario: Connect-back UDF reaches a routable database endpoint without crashing the session

* *GIVEN* a registered slim SLC session and a deployed connect-back UDF registered with `%connection CB_SELF`
* *AND* `CB_SELF` is created `TO '<routable-endpoint>:8563'` pointing at an address reachable from the UDF sandbox network namespace
* *WHEN* the UDF is invoked and its `run` opens a connect-back connection and runs a query
* *THEN* the connect-back connection MUST succeed and return results to the UDF
* *AND* the parent database session MUST remain alive throughout — it MUST NOT be terminated by a server-side signal
* *AND* the query MUST complete against the live `2026.1.0` container
<!-- /NEW -->
