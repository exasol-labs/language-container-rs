# Feature: host-dispatch

Orchestrates loading a UDF `.so`, building the host-side `UdfContext` bridge, and dispatching the database execution model — scalar/set run loops, single-call functions, and connect-back — over the wire protocol.

## Background

Connect-back opens a connection from inside the UDF sandbox back to Exasol. The host fetches credentials via `MT_IMPORT` (`PB_IMPORT_CONNECTION_INFORMATION`) — a pure metadata retrieval equivalent to PyExasol's `exa.get_connection(NAME)` — then connects to the returned `address` as an ordinary external client over the exarrow-rs native binary protocol with server-certificate validation disabled.

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Connect-back connects to the named connection address like an external client

* *GIVEN* a `connection_information_rep` whose `address` is a routable Exasol endpoint and whose `kind` is `password`
* *WHEN* the host opens the connect-back connection
* *THEN* it MUST connect to the `address` exactly as an ordinary external client would, opening a new database session and a new transaction, authenticated with the `user` and `password` from the response and not a session token
* *AND* it MUST NOT attempt to share or join the invoking query's session or transaction, because the Exasol core cannot share a transaction with a language-container UDF
* *AND* it MUST disable server-certificate validation to match the project transport rule
* *AND* it MUST use the exarrow-rs native binary protocol by relying on the default `native` feature, and MUST NOT set a `transport=websocket` override in the DSN
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: Connect-back named connection makes the UDF portable across clusters

* *GIVEN* a UDF script registered with a generic `%connection <NAME>` directive and no hardcoded cluster address in its source
* *AND* a database `CREATE CONNECTION <NAME> TO '<cluster-address>:8563' USER '...' IDENTIFIED BY '...'` object that the operator populated with the correct address for the target cluster
* *WHEN* the host requests credentials with `MT_IMPORT` (`PB_IMPORT_CONNECTION_INFORMATION`) naming `<NAME>` and receives the `connection_information_rep`
* *THEN* the host MUST build the connect-back DSN solely from the `address`, `user`, and `password` returned for that named connection
* *AND* the host MUST NOT embed or assume any cluster-specific address of its own, so the same UDF artifact remains portable across clusters that differ only in the `CREATE CONNECTION` definition
<!-- /DELTA:NEW -->
