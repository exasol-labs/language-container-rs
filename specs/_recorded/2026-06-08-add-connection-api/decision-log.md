# Decision Log: add-connection-api

Date: 2026-06-07

## Interview

**Q:** What should `connection(name)` return?
**A:** A raw struct — `ConnectionObject { kind, address, user, password }`. UDF authors use it as they see fit.

**Q:** Where do credentials for the cluster-IP path come from?
**A:** The UDF author passes them explicitly. They may obtain `user`/`password` from `ctx.connection("CREDS_CONN")` and ignore its `address`, then pair those credentials with `ctx.cluster_ip()` to target the cluster node directly.

**Q:** What is the API shape?
**A:** Three methods on `UdfContext`, feature-gated behind `connect-back`:
- `fn cluster_ip(&self) -> Result<String, UdfError>`
- `fn connection(&self, name: &str) -> Result<ConnectionObject, UdfError>`
- `fn connect_back(&mut self, conn: &ConnectionObject) -> Result<Box<dyn ExaConnection>, UdfError>`

**Q:** Three concepts must not be confused — which is which?
**A:** Per CLAUDE.md: the Exasol CONNECTION object → `ConnectionObject` (credential store fetched via MT_IMPORT); the exarrow-rs session → `Box<dyn ExaConnection>` (live ADBC session, new transaction); the cluster node IP → `ctx.cluster_ip()` (parsed from the ZMQ endpoint `args[1]`). A UDF may also use a `ConnectionObject` to reach a foreign system with its own driver; `connect_back` is the Exasol-specific convenience on top.

## Design Decisions

### [1] ConnectionObject is a public SDK type; ConnInfo stays internal to the protocol layer

- **Decision:** Add a public `ConnectionObject { kind, address, user, password }` struct in `exasol-udf-sdk::connect_back`. The protocol-layer `exa-zmq-protocol::ConnInfo` (same fields) remains internal; the runtime bridge maps `ConnInfo` ↔ `ConnectionObject` at the boundary.
- **Alternatives:** Re-export `ConnInfo` from the SDK so the two are one type. Rejected — it would pull a transport-layer crate into the SDK's public surface and couple the author-facing API to wire-format evolution.
- **Rationale:** A dedicated DTO keeps the SDK free of transport dependencies (the feature-gate scenario already forbids `tokio`/`exarrow-rs`), lets the public field set evolve independently, and lets authors construct a `ConnectionObject` directly for foreign systems.
- **Intended ADR:** ADR-016
- **Promotes to ADR:** yes

### [2] cluster_ip() returns the raw node IP, with no port appended

- **Decision:** `cluster_ip()` returns `<node_ip>` parsed from the ZMQ endpoint `tcp://<node_ip>:<zmq_port>` by stripping `tcp://` and taking the host segment before `:`. It does not append `:8563` or the ZMQ port.
- **Alternatives:** Return the SQL endpoint `<node_ip>:8563`. Rejected — the author chooses the port (SQL is 8563 by default but may differ), and a raw IP composes cleanly with credentials from `connection` and any target port.
- **Rationale:** Keeps the method a pure parse with a single responsibility; the ZMQ port is not the SQL port, so appending the wrong port would be misleading.
- **Intended ADR:** ADR-017
- **Promotes to ADR:** yes

### [3] connection(name) performs an on-demand MT_IMPORT during the blocked dispatch loop

- **Decision:** `connection(name)` sends `MT_IMPORT` (`PB_IMPORT_CONNECTION_INFORMATION`, `script_name = name`) synchronously while the outer dispatch loop is blocked awaiting the UDF function return, then maps the `connection_information_rep` into a `ConnectionObject`. It does NOT open any session.
- **Alternatives:** Require all needed connection names to be declared in the `%connection` script header and fetched at handshake (MT_META phase). Rejected — the protocol already parses `MT_IMPORT` in any phase (`(MessageType::MtImport, _)` in `loop_.rs`), and the existing `conn_requester` closure in `run_batch` already proves run-phase MT_IMPORT is safe because the ZMQ socket is idle while the UDF executes. Run-time fetch is also more flexible (the name need not be known at registration).
- **Rationale:** Reuses a proven, already-shipped mechanism; generalises the existing single-shot closure to take the connection name; no protocol changes needed.
- **Intended ADR:** ADR-018 (MT_IMPORT timing during MT_RUN)
- **Promotes to ADR:** yes

### [4] Remove exa()/exa_named()/exa_connect()/ConnectBackOptions; clean break

- **Decision:** Delete the old three connect-back methods and the `ConnectBackOptions` enum rather than keep them as deprecated aliases. The new `cluster_ip`/`connection`/`connect_back` surface fully supersedes them.
- **Alternatives:** Keep the old methods as deprecated shims. Rejected — this is a 0.x library, two parallel APIs add maintenance burden and confusion, and the old lazy-default `exa()` (implicit handshake-credential connection) has no equivalent in the explicit new model.
- **Rationale:** A single coherent surface; the implicit default connection and its credential caching are removed in favour of explicit author-driven connection.
- **Scope:** local API-surface decision captured by the spec deltas
- **Promotes to ADR:** no

### [5] connect_back returns an owned Box per call; no single-connection cache

- **Decision:** `connect_back(&ConnectionObject)` returns a fresh `Box<dyn ExaConnection>` the UDF owns; the bridge no longer caches a single default connection.
- **Alternatives:** Cache one connection per bridge like the old `exa()`. Rejected — caching only made sense for the lazy no-arg default; with explicit `ConnectionObject` arguments a UDF may legitimately open several connections (e.g. cluster node + a foreign system) in one call.
- **Scope:** implementation detail of the bridge
- **Promotes to ADR:** no

## Review Findings

<!-- Populated by speq-implement after code review. -->
