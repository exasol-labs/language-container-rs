# Feature: db-roundtrip

Exercises the full Rust SLC against a live Exasol container — registering the slim SLC, uploading the UDF artifact to BucketFS, and invoking UDFs end-to-end to prove the wire protocol, dispatch, and SDK behave correctly against a real database.

## Background

The integration harness starts an `exasol/docker-db:2026.latest` container (identical image to `2026.1.0` at time of writing), registers the slim SLC, and uploads UDF `.so` artifacts to BucketFS with SSL verification disabled per project rules. Connect-back scenarios create a `CB_SELF` connection `TO '<routable-endpoint>:8563'` and register UDFs with a generic `%connection CB_SELF` directive. On `2026.latest` the connect-back round-trip is blocked by a server-side SIGABRT (signal 6) that terminates the invoking session when the core spawns the connect-back session process; those scenarios are therefore known-failing until a patched image ships.

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Harness starts Exasol and connects

* *GIVEN* a Docker daemon with the `exasol/docker-db:2026.latest` image available
* *WHEN* the harness starts the container in privileged mode and waits for readiness
* *THEN* the database port `8563` and BucketFS port `2581` MUST be mapped to host ports
* *AND* an `exarrow-rs` connection to the mapped DB port MUST succeed and return a non-empty result for `SELECT 1`
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: Connect-back UDF queries the database and emits the result

* *GIVEN* a registered slim SLC session and a deployed connect-back example UDF whose `run` calls `ctx.exa()?.query_arrow("SELECT 42")`
* *AND* a `CB_SELF` connection created `TO '<routable-endpoint>:8563'` reachable from the UDF sandbox, supplying the address/user/password the UDF connects with as an external client
* *WHEN* the UDF is invoked over the live Exasol container
* *THEN* on a supported database build the UDF MUST receive the query result as Arrow batches via a new external client session and emit the value `42`
* *AND* the connect-back session MUST be a new session and a new transaction, distinct from the invoking query's session
* *AND* on `exasol/docker-db:2026.latest` the harness MUST surface the documented server-side connect-back SIGABRT as a known failure with the connect-back diagnostic logs rather than masking it
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: Connect-back DML UDF inserts rows and data is visible externally

* *GIVEN* a registered slim SLC session and a deployed `connect-back-insert` UDF
* *AND* an input table containing three `BIGINT` rows `[10, 20, 30]`
* *AND* a `CB_SELF` connection created `TO '<routable-endpoint>:8563'` reachable from the UDF sandbox
* *WHEN* the UDF is invoked and its `run` creates `cb_result` via a new external client connect-back session and inserts each input value
* *THEN* on a supported database build `exapump` MUST be able to `SELECT val FROM cb_result ORDER BY val` against the same container and return exactly `[10, 20, 30]`, connecting with `validateservercertificate=0`
* *AND* on `exasol/docker-db:2026.latest` the harness MUST treat this scenario as a known failure for the same server-side connect-back SIGABRT as the connect-back query scenario
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: Connect-back UDF reaches a routable database endpoint without crashing the session

* *GIVEN* a registered slim SLC session and a deployed connect-back UDF registered with a generic `%connection CB_SELF` directive
* *AND* `CB_SELF` is created `TO '<routable-endpoint>:8563'` pointing at an address reachable from the UDF sandbox network namespace (the Docker host gateway plus the host-mapped DB port)
* *WHEN* the UDF is invoked and its `run` opens a connect-back connection and runs a query
* *THEN* on a patched database build the connect-back connection MUST succeed and the parent database session MUST remain alive throughout
* *AND* on `exasol/docker-db:2026.latest` the scenario MUST be treated as a known failure caused by a documented upstream defect — the parent session is terminated by signal 6 (core dumped) the moment the core spawns the connect-back session process, independent of the connect-back address or transport — and MUST NOT be attributed to the SLC until a patched `2026.x` image is published
<!-- /DELTA:CHANGED -->
