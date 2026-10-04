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
