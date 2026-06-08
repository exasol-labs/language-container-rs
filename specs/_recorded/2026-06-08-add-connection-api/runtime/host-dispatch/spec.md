# Feature: host-dispatch

Orchestrates loading a UDF `.so`, building the host-side `UdfContext` bridge, and dispatching the database execution model — scalar/set run loops, single-call functions, and connect-back — over the wire protocol.

## Background

Connect-back opens a connection from inside the UDF sandbox back to Exasol. The connect-back surface is three composable `UdfContext` methods: `cluster_ip()` parses the originating node IP from the ZMQ endpoint with no network call; `connection(name)` retrieves the raw credentials of a named database `CONNECTION` object via an on-demand `MT_IMPORT` (`PB_IMPORT_CONNECTION_INFORMATION`) exchange and returns a `ConnectionObject`; `connect_back(&ConnectionObject)` opens a live `exarrow-rs` session to the target as an ordinary external client over the native binary protocol with server-certificate validation disabled. The MT_IMPORT exchange is safe during the run phase because the outer dispatch loop is blocked awaiting the UDF function return, so the ZMQ socket is idle.

## Scenarios

<!-- DELTA:NEW -->
### Scenario: cluster_ip is parsed from the ZMQ endpoint without a network call

* *GIVEN* a runtime built with the `connect-back` feature started with the ZMQ endpoint `tcp://<node_ip>:<zmq_port>` (the `args[1]` the database passes to the container)
* *WHEN* a UDF calls `ctx.cluster_ip()`
* *THEN* the `HostContextBridge` MUST return `<node_ip>` parsed from the endpoint by stripping the `tcp://` scheme prefix and taking the host segment before the `:`
* *AND* it MUST NOT append the `:8563` SQL port or any port to the returned value, leaving port selection to the UDF author
* *AND* it MUST NOT perform any network round-trip to obtain the IP, because the endpoint string already names the originating node
* *AND* an endpoint that does not parse into a host segment MUST return `UdfError::ConnectBack` rather than panicking
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: connection fetches named-connection credentials via on-demand MT_IMPORT

* *GIVEN* a runtime built with the `connect-back` feature, where the outer dispatch loop is blocked awaiting the UDF function return so the ZMQ socket is idle
* *WHEN* a UDF calls `ctx.connection("CREDS_CONN")` during `run_batch`
* *THEN* the host MUST send an `MT_IMPORT` request with `kind = PB_IMPORT_CONNECTION_INFORMATION` and `script_name = "CREDS_CONN"`
* *AND* it MUST map the returned `connection_information_rep` (`kind`, `address`, `user`, `password`) into a public `ConnectionObject` and return it to the UDF
* *AND* it MUST NOT open any `exarrow-rs` session as a side effect, because `connection` is a pure credential retrieval
* *AND* a database error or a non-`ConnInfo` reply MUST be returned as `UdfError::ConnectBack` rather than panicking
<!-- /DELTA:NEW -->

<!-- DELTA:CHANGED -->
### Scenario: Connect-back opens a connection from a ConnectionObject

* *GIVEN* a `HostContextBridge` built with the `connect-back` feature and a `ConnectionObject` carrying an `address`, `user`, and `password`
* *WHEN* a UDF calls `ctx.connect_back(&conn)`
* *THEN* the `HostContextBridge` MUST open an `exarrow-rs` connection on the dedicated `CONNECT_BACK_RT` runtime using the `address`, `user`, and `password` of the passed `ConnectionObject`
* *AND* it MUST return the session as a `Box<dyn ExaConnection>` owned by the UDF, so the author MAY open more than one connection within a single call
* *AND* it MUST NOT consult the handshake credentials or send an MT_IMPORT request, because the `ConnectionObject` already carries the target credentials
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: Connect-back query returns Arrow batches to the UDF

* *GIVEN* a `Box<dyn ExaConnection>` returned by `ctx.connect_back`
* *WHEN* the UDF calls `query_arrow` with a SELECT statement
* *THEN* the host MUST execute the query on the `CONNECT_BACK_RT` runtime and return the result as `Vec<RecordBatch>`
* *AND* a query failure MUST be returned as `UdfError::ConnectBack` rather than panicking
<!-- /DELTA:CHANGED -->

<!-- DELTA:REMOVED -->
### Scenario: Connect-back opens a connection from the handshake credentials

* *GIVEN* a runtime built with the `connect-back` feature where `UdfMeta` carries connection information from `MT_INFO`
* *WHEN* a UDF first calls `ctx.exa()`
* *THEN* the lazy default `exa()` connection MUST be removed, because there is no implicit default connection in the new explicit `connection`/`connect_back` model
<!-- /DELTA:REMOVED -->

<!-- DELTA:REMOVED -->
### Scenario: Connect-back retrieves credentials on demand when the handshake carries none

* *GIVEN* a runtime built with the `connect-back` feature where `UdfMeta.conn_info` is `None`
* *WHEN* a UDF first calls `ctx.exa()`
* *THEN* the on-demand fetch path for the implicit default connection MUST be removed, because credential retrieval now happens explicitly through `ctx.connection(name)`
<!-- /DELTA:REMOVED -->

<!-- DELTA:CHANGED -->
### Scenario: Connect-back connects to the named connection address like an external client

* *GIVEN* a `ConnectionObject` whose `address` is a routable Exasol endpoint and whose `kind` is `password`, obtained from `ctx.connection(<name>)`
* *WHEN* the host opens the connect-back connection via `ctx.connect_back`
* *THEN* it MUST connect to the `address` exactly as an ordinary external client would, opening a new database session and a new transaction, authenticated with the `user` and `password` from the object and not a session token
* *AND* it MUST NOT attempt to share or join the invoking query's session or transaction, because the Exasol core cannot share a transaction with a language-container UDF
* *AND* it MUST disable server-certificate validation to match the project transport rule
* *AND* it MUST use the exarrow-rs native binary protocol by relying on the default `native` feature, and MUST NOT set a `transport=websocket` override in the DSN
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: Connect-back named connection makes the UDF portable across clusters

* *GIVEN* a UDF that calls `ctx.connection(<NAME>)` for a generic connection name with no hardcoded cluster address in its source
* *AND* a database `CREATE CONNECTION <NAME> TO '<cluster-address>:8563' USER '...' IDENTIFIED BY '...'` object that the operator populated with the correct address for the target cluster
* *WHEN* the UDF passes the returned `ConnectionObject` to `ctx.connect_back`
* *THEN* the host MUST build the connect-back DSN solely from the `address`, `user`, and `password` of that `ConnectionObject`
* *AND* the host MUST NOT embed or assume any cluster-specific address of its own, so the same UDF artifact remains portable across clusters that differ only in the `CREATE CONNECTION` definition
<!-- /DELTA:CHANGED -->
