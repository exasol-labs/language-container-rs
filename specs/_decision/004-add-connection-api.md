# Decisions: add-connection-api

## ADR: ConnectionObject is a public SDK type; ConnInfo stays internal to the protocol layer

**ID:** connectionobject-public-sdk-type
**Plan:** `add-connection-api`
**Status:** Accepted

### Context

UDF authors need a public credential struct. `exa-zmq-protocol::ConnInfo` has the same four fields (`kind`, `address`, `user`, `password`) but belongs to the wire layer.

### Decision

`exasol-udf-sdk::connect_back` defines the public `ConnectionObject { kind, address, user, password }`. `ConnInfo` stays internal, and the runtime bridge maps `ConnInfo` and `ConnectionObject` at the boundary.

### Options Considered

| Option | Verdict |
|--------|---------|
| Dedicated public `ConnectionObject` | ✓ Chosen, SDK stays free of transport dependencies |
| Re-export `ConnInfo` | ✗ Couples the author-facing API to wire-format changes |

### Consequences

Authors can construct a `ConnectionObject` for foreign systems without `MT_IMPORT`. The feature-gate scenario forbids `tokio`/`exarrow-rs` in the SDK.

## ADR: connection(name) performs an on-demand MT_IMPORT during the blocked dispatch loop

**ID:** connection-name-on-demand-mt-import
**Plan:** `add-connection-api`
**Status:** Accepted

### Context

`connection(name)` must retrieve raw credentials for a named database `CONNECTION` object. Fetching at handshake would require every name in the `%connection` header.

### Decision

`connection(name)` sends `MT_IMPORT` (`PB_IMPORT_CONNECTION_INFORMATION`, `script_name = name`) synchronously while the outer dispatch loop is blocked awaiting the UDF function return, then maps the `connection_information_rep` into a `ConnectionObject`. It opens no session.

### Options Considered

| Option | Verdict |
|--------|---------|
| On-demand `MT_IMPORT` during `run_batch` | ✓ Chosen, name need not be known at registration |
| Fetch all connections at handshake (MT_META) | ✗ Requires all names in the `%connection` header |

### Consequences

The `conn_requester` closure takes the connection name as a parameter. The ZMQ socket is idle while the UDF function runs, so the synchronous exchange is safe.
