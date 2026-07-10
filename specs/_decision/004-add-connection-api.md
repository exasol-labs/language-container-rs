# Decisions: add-connection-api

## ADR: ConnectionObject is a public SDK type; ConnInfo stays internal to the protocol layer

**ID:** connectionobject-public-sdk-type
**Plan:** `add-connection-api`
**Status:** Accepted

### Context

The connect-back API needs a public credential struct for UDF authors. The protocol layer already has `exa-zmq-protocol::ConnInfo` with the same four fields (`kind`, `address`, `user`, `password`). A decision was needed on whether to re-export `ConnInfo` from the SDK or introduce a dedicated public type.

### Decision

Add a public `ConnectionObject { kind, address, user, password }` struct in `exasol-udf-sdk::connect_back`. The protocol-layer `ConnInfo` remains internal; the runtime bridge maps `ConnInfo` ↔ `ConnectionObject` at the boundary.

### Options Considered

| Option | Verdict |
|--------|---------|
| Dedicated public DTO (`ConnectionObject`) in the SDK | ✓ Chosen — keeps the SDK free of transport dependencies; lets the public field set evolve independently; lets authors construct a `ConnectionObject` directly for foreign systems |
| Re-export `ConnInfo` from the SDK | ✗ Rejected — pulls `exa-zmq-protocol` into the SDK's public surface and couples the author-facing API to wire-format evolution |

### Consequences

A dedicated DTO keeps the SDK free of transport dependencies (the feature-gate scenario already forbids `tokio`/`exarrow-rs`). Authors can construct a `ConnectionObject` directly for foreign systems without going through `MT_IMPORT`. The runtime bridge performs the `ConnInfo` ↔ `ConnectionObject` mapping at the ZMQ boundary.

## ADR: cluster_ip() returns the raw node IP with no port appended

**ID:** cluster-ip-returns-raw-node-ip-no-port
**Plan:** `add-connection-api`
**Status:** Accepted

### Context

`cluster_ip()` parses the originating node IP from the ZMQ endpoint string `tcp://<node_ip>:<zmq_port>`. A decision was needed on whether to return the raw IP or to append the well-known SQL port `:8563`.

### Decision

`cluster_ip()` returns `<node_ip>` by stripping `tcp://` and taking the host segment before `:`. It does not append `:8563` or the ZMQ port.

### Options Considered

| Option | Verdict |
|--------|---------|
| Return raw `<node_ip>`, no port | ✓ Chosen — authors choose the port; raw IP composes cleanly with credentials from `connection` and any target port; the ZMQ port is not the SQL port |
| Return `<node_ip>:8563` | ✗ Rejected — the SQL port may differ from the default; appending the wrong port would be misleading; breaks the single-responsibility of the parse |

### Consequences

Authors receive a bare IP string and select the port themselves. The method is a pure parse with no network round-trip. A UDF may pair `cluster_ip()` with credentials from `connection()` and supply any port when building a DSN.

## ADR: connection(name) performs an on-demand MT_IMPORT during the blocked dispatch loop

**ID:** connection-name-on-demand-mt-import
**Plan:** `add-connection-api`
**Status:** Accepted

### Context

`connection(name)` must retrieve raw credentials for a named database `CONNECTION` object. Two timing options were considered: fetch at handshake (MT_META phase, requiring all names to be declared in the `%connection` header) or fetch on demand during `run_batch`.

### Decision

`connection(name)` sends `MT_IMPORT` (`PB_IMPORT_CONNECTION_INFORMATION`, `script_name = name`) synchronously while the outer dispatch loop is blocked awaiting the UDF function return, then maps the `connection_information_rep` into a `ConnectionObject`. It does not open any session.

### Options Considered

| Option | Verdict |
|--------|---------|
| On-demand MT_IMPORT during `run_batch` (blocked dispatch loop) | ✓ Chosen — protocol already parses `MT_IMPORT` in any phase; existing `conn_requester` closure proves run-phase exchange is safe; name need not be known at registration |
| Fetch all connections at handshake (MT_META phase) | ✗ Rejected — requires all connection names to be declared in the `%connection` header at script registration time; less flexible |

### Consequences

The generalised `conn_requester` closure takes the connection name as a parameter. The ZMQ socket is idle while the UDF function executes, making the synchronous MT_IMPORT exchange safe. No protocol changes are needed; the run-phase MT_IMPORT path was already proven by the existing implementation.
