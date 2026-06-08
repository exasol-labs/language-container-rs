# Feature: connect-back

Exercises the three-method connect-back API (`cluster_ip`, `connection`, `connect_back`) end-to-end against a live Exasol container. `cluster_ip` is a pure parse and passes on 2026.latest. The connect-back query and DML scenarios open an actual exarrow-rs external-client session and remain KNOWN_FAILING on 2026.latest until the upstream server-side SIGABRT is patched.

## Background

The integration harness starts an `exasol/docker-db:2026.latest` container, registers the slim SLC, and uploads UDF `.so` artifacts to BucketFS with SSL verification disabled per project rules. Connect-back scenarios create a `CB_SELF` connection `TO '<routable-endpoint>:8563'` and register UDFs with a generic `%connection CB_SELF` directive. On `2026.latest` the connect-back round-trip is blocked by a server-side SIGABRT (signal 6) that terminates the invoking session when the core spawns the connect-back session process; those scenarios are therefore known-failing until a patched image ships.

## Scenarios

### Scenario: cluster_ip UDF emits the node IP without opening a connect-back session

* *GIVEN* a registered slim SLC session and a deployed scalar UDF that calls `ctx.cluster_ip()` and emits the returned string
* *WHEN* the UDF is invoked over the live Exasol 2026.latest container
* *THEN* the query MUST return a non-empty string
* *AND* the returned value MUST be a valid IPv4 address (four dot-separated octets, no port suffix)
* *AND* the scenario MUST pass as a hard assertion on 2026.latest, because `cluster_ip()` performs no network round-trip and does not open any connect-back session — there is no Part:44 spawn that triggers the ADR-015 SIGABRT

### Scenario: Connect-back UDF queries the database and emits the result

* *GIVEN* a registered slim SLC session and a deployed connect-back query UDF that calls `ctx.connection("CB_SELF")` and then `ctx.connect_back(&conn_obj)` using the three-method API
* *AND* a `CB_SELF` connection created `TO '<docker-host-gateway>:<mapped-port>'` reachable from the UDF sandbox, supplying the address/user/password the UDF connects with as an external client
* *WHEN* the UDF is invoked over the live Exasol container
* *THEN* on a supported database build the UDF MUST receive the query result as Arrow batches via a new external client session and emit the value
* *AND* the connect-back session MUST be a new session and a new transaction, distinct from the invoking query's session
* *AND* on `2026.latest` the harness MUST surface the documented server-side connect-back SIGABRT as a known failure with the connect-back diagnostic logs rather than masking it

### Scenario: Connect-back DML UDF inserts rows and data is visible externally

* *GIVEN* a registered slim SLC session and a deployed connect-back insert UDF that calls `ctx.connection("CB_SELF")` and then `ctx.connect_back(&conn_obj)` using the three-method API
* *AND* an input table containing three `BIGINT` rows
* *AND* a `CB_SELF` connection created `TO '<address>'` reachable from the UDF sandbox
* *WHEN* the UDF is invoked and its `connect_back` creates a new external client session and inserts each input value
* *THEN* on a supported database build `exapump` MUST be able to `SELECT val FROM cb_result ORDER BY val` against the same container and return exactly `10`, `20`, `30`, connecting with `validateservercertificate=0`
* *AND* on `2026.latest` the harness MUST treat this scenario as a known failure for the same server-side connect-back SIGABRT as the connect-back query scenario

### Scenario: Connect-back UDF reaches a routable database endpoint without crashing the session

* *GIVEN* a registered slim SLC session and a deployed connect-back UDF registered with a generic `%connection CB_SELF` directive
* *AND* `CB_SELF` is created `TO '<routable-endpoint>:8563'` pointing at an address reachable from the UDF sandbox network namespace (the Docker host gateway plus the host-mapped DB port)
* *WHEN* the UDF is invoked and its `run` opens a connect-back connection and runs a query
* *THEN* on a patched database build the connect-back connection MUST succeed and the parent database session MUST remain alive throughout
* *AND* on `exasol/docker-db:2026.latest` the scenario MUST be treated as a known failure caused by a documented upstream defect — the parent session is terminated by signal 6 (core dumped) the moment the core spawns the connect-back session process, independent of the connect-back address or transport — and MUST NOT be attributed to the SLC until a patched `2026.x` image is published
