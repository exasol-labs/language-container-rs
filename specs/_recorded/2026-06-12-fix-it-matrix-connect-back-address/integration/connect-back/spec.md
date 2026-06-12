# Feature: connect-back

Exercises the three-method connect-back API (`cluster_ip`, `connection`, `connect_back`) end-to-end against a live Exasol container across the supported version matrix. The connect-back query and DML scenarios open an actual `exarrow-rs` external-client session over TCP to the cluster node's own SQL endpoint and prove a full round-trip, exactly as any regular Exasol client would connect.

## Background

The integration harness starts an `exasol/docker-db:<version>` container (version selected by the `EXASOL_VERSION` env var, default `2026.1.0`), registers the slim SLC, and uploads UDF `.so` artifacts to BucketFS with SSL verification disabled per project rules. Connect-back scenarios create a `CB_SELF` connection whose address is returned by `connect_back_sql_address()`.

`connect_back_sql_address()` ALWAYS returns `<container-eth0-ip>:8563` — the DB container's own `eth0` IP with the internal SQL port — in BOTH testcontainers mode and external mode. This address is obtained via `container_inner_ip()`, which runs `docker exec` into the named DB container (`exasol-db`) to read its primary IPv4. The connect-back UDF runs inside the DB container's own network namespace; `<container-eth0-ip>:8563` is the node's real SQL listener and is reachable from that namespace exactly as any regular external client would connect.

`localhost` / `127.0.0.1:8563` MUST NEVER be used as the connect-back address. That loopback path routes to Exasol's internal CoreDB proxy, which links the new connect-back session to the invoking SQL worker (Part:40) rather than creating an independent external-client session. The result is a VM SIGABRT within seconds; on a shared connection this crash poisons the entire session and every subsequent scenario fails. This was the root cause of the IT-matrix failure in external mode, where `localhost:8563` was previously used.

A genuine remote cluster (no local Docker container to `exec` into) is out of scope; an explicit `EXASOL_CB_ADDRESS` environment-variable override would cover that future use case. The IT suite's external mode always targets a local Docker container named `exasol-db` in both CI and local-repro configurations, so `container_inner_ip()` is always applicable.

UDFs are registered with a generic `%connection CB_SELF` directive. The connect-back act opens a brand-new `exarrow-rs` session over TCP against the `<container-eth0-ip>:8563` endpoint, with its own transaction independent of the invoking query.

## Scenarios

### Scenario: cluster_ip UDF emits the node IP

* *GIVEN* a registered slim SLC session and a deployed scalar UDF that calls `ctx.cluster_ip()` and emits the returned string
* *WHEN* the UDF is invoked over the live Exasol container
* *THEN* the query MUST return a non-empty string that is a valid IPv4 address (four dot-separated octets, no port suffix)
* *AND* `cluster_ip()` MUST derive the address from the local node's primary network interface (the first non-loopback IPv4 of the UDF process, e.g. the container `eth0` address) rather than from parsing the ZMQ endpoint string, so it returns a valid IPv4 on both single-node Docker and multi-node TCP deployments
* *AND* the harness MUST assert the IPv4 result as a hard assertion on every version in the matrix (`2025.1`, `2025.2`, `2026.1`) — there is NO severity branch and NO unconditional skip

### Scenario: Connect-back UDF queries the database and emits the result

* *GIVEN* a registered slim SLC session and a deployed connect-back query UDF that calls `ctx.connection("CB_SELF")` and then `ctx.connect_back(&conn_obj)` using the three-method API
* *AND* a `CB_SELF` connection created `TO '<connect_back_sql_address()>'` — always `<container-eth0-ip>:8563` in both testcontainers and external mode (a direct TCP path to the node's SQL listener, never loopback) — supplying the address/user/password the UDF connects with as an external client
* *WHEN* the UDF is invoked over the live Exasol container
* *THEN* the UDF MUST receive the result of `SELECT 42` via a new external-client session and emit `42`
* *AND* the UDF MUST read the result through the FFI-safe `ExaConnection::query()` method, which returns rows of the SDK `Value` type (the arrow→Value conversion runs inside the runtime). The UDF MUST NOT downcast raw Arrow arrays returned by `query_arrow()` across the cdylib boundary — those carry the runtime's Arrow `TypeId`, which does not match the UDF `.so`'s own Arrow copy, so `downcast_ref` silently fails
* *AND* the emitted `BIGINT` value MUST be sent as `Value::Numeric` (Exasol delivers/expects `BIGINT` as `PB_NUMERIC`)
* *AND* the connect-back session MUST be a new session and a new transaction, distinct from the invoking query's session, and the invoking query's session MUST remain alive throughout
* *AND* the harness MUST assert this as a hard assertion on every version in the matrix (`2025.1`, `2025.2`, `2026.1`)

### Scenario: Connect-back DML UDF inserts rows and data is visible externally

* *GIVEN* a registered slim SLC session and a deployed connect-back insert UDF that calls `ctx.connection("CB_SELF")` and then `ctx.connect_back(&conn_obj)` using the three-method API
* *AND* an input table containing three `BIGINT` rows (`10`, `20`, `30`)
* *AND* a target table `cb_sink.cb_result` created and committed in a SEPARATE schema BEFORE the invoking query runs (the connect-back session is a distinct transaction that can only see committed objects; writing to or creating objects in the invoking query's own schema would force the invoking transaction into WAIT FOR COMMIT under Serializable isolation and trigger the deadlock detector)
* *AND* a `CB_SELF` connection created `TO '<connect_back_sql_address()>'` — always `<container-eth0-ip>:8563` in both testcontainers and external mode (a direct TCP path to the node's SQL listener, never loopback)
* *WHEN* the UDF is invoked and its `connect_back` creates a new external-client session and inserts each input value into `cb_sink.cb_result` (no DDL and no explicit `COMMIT` — the connect-back session autocommits)
* *THEN* `exapump` MUST be able to `SELECT val FROM cb_sink.cb_result ORDER BY val` against the same container and return exactly `10`, `20`, `30`, connecting with `validateservercertificate=0`
* *AND* the connect-back session's transaction MUST commit independently of the invoking query's transaction, asserted as a hard assertion on every version in the matrix

### Scenario: Connect-back write-back into a pre-committed table in the invoking schema

* *GIVEN* a registered slim SLC session and a deployed connect-back SET UDF that number-crunches each input value (squares it) and connect-back-inserts the pair `(v, v*v)`
* *AND* a target table `it_rust.crunch_log` created and seeded (committed) in the invoking query's OWN schema BEFORE the query runs
* *AND* a SEPARATE input table `it_rust.crunch_in` holding `2`, `3`, `4`, which the invoking query reads (it never reads `crunch_log`, so there is no read-write conflict between the two transactions)
* *AND* a `CB_SELF` connection created `TO '<connect_back_sql_address()>'`
* *WHEN* the UDF is invoked as `SELECT crunch_writeback(v) FROM it_rust.crunch_in`, and afterwards a brand-new independent session inserts one more row into `it_rust.crunch_log`
* *THEN* `exapump` MUST be able to `SELECT v_squared FROM it_rust.crunch_log ORDER BY v` and return the seeded row, the three UDF-written squares, and the post-UDF row (`1, 4, 9, 16, 25`)
* *AND* same-schema write-back MUST succeed without a transaction-conflict abort, demonstrating that Serializable isolation is satisfied when the target is pre-committed, the UDF performs no DDL and no explicit `COMMIT` (autocommit), and the invoking query reads a different object than the UDF writes
