# Feature: db-roundtrip

End-to-end scenarios that start a real `exasol/docker-db:2026.latest` container, register the slim Rust SLC, upload precompiled musl UDF `.so` files to BucketFS, and assert query results over a live `exarrow-rs` connection. Gated behind the `integration` feature.

## Background

The three-method connect-back API (`cluster_ip`, `connection`, `connect_back`) is exercised end-to-end against the live container. `cluster_ip()` is a pure parse — it does not open a connect-back session and is therefore not blocked by the ADR-015 server-side SIGABRT on 2026.latest. The connect-back query and DML scenarios open an actual exarrow-rs external-client session and remain KNOWN_FAILING on 2026.latest until the upstream SIGABRT is patched.

## Scenarios

<!-- DELTA:NEW -->
### Scenario: cluster_ip UDF emits the node IP without opening a connect-back session

* *GIVEN* a registered slim SLC session and a deployed scalar UDF that calls `ctx.cluster_ip()` and emits the returned string
* *WHEN* the UDF is invoked over the live Exasol 2026.latest container
* *THEN* the query MUST return a non-empty string
* *AND* the returned value MUST be a valid IPv4 address (four dot-separated octets, no port suffix)
* *AND* the scenario MUST pass as a hard assertion on 2026.latest, because `cluster_ip()` performs no network round-trip and does not open any connect-back session — there is no Part:44 spawn that triggers the ADR-015 SIGABRT
<!-- /DELTA:NEW -->

<!-- DELTA:CHANGED -->
### Scenario: Connect-back UDF queries the database and emits the result

* *GIVEN* a registered slim SLC session and a deployed connect-back query UDF that calls `ctx.connection("CB_SELF")` and then `ctx.connect_back(&conn_obj)` using the three-method API
* *AND* a `CB_SELF` connection created `TO '<docker-host-gateway>:<mapped-port>'` reachable from the UDF sandbox, supplying the address/user/password the UDF connects with as an external client
* *WHEN* the UDF is invoked over the live Exasol container
* *THEN* on a supported database build the UDF MUST receive the query result as Arrow batches via a new external client session and emit the value
* *AND* the connect-back session MUST be a new session and a new transaction, distinct from the invoking query's session
* *AND* on `2026.latest` the harness MUST surface the documented server-side connect-back SIGABRT as a known failure with the connect-back diagnostic logs rather than masking it
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: Connect-back DML UDF inserts rows and data is visible externally

* *GIVEN* a registered slim SLC session and a deployed connect-back insert UDF that calls `ctx.connection("CB_SELF")` and then `ctx.connect_back(&conn_obj)` using the three-method API
* *AND* an input table containing three `BIGINT` rows
* *AND* a `CB_SELF` connection created `TO '<address>'` reachable from the UDF sandbox
* *WHEN* the UDF is invoked and its `connect_back` creates a new external client session and inserts each input value
* *THEN* on a supported database build `exapump` MUST be able to `SELECT val FROM cb_result ORDER BY val` against the same container and return exactly `10`, `20`, `30`, connecting with `validateservercertificate=0`
* *AND* on `2026.latest` the harness MUST treat this scenario as a known failure for the same server-side connect-back SIGABRT as the connect-back query scenario
<!-- /DELTA:CHANGED -->
