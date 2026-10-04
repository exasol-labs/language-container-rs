# Decisions: add-v2-rust-udf-complete

## ADR: ExaConnection trait in the SDK, implemented by the runtime

**ID:** exaconnection-trait-in-sdk
**Plan:** `add-v2-rust-udf-complete`
**Status:** Accepted

### Context

Exposing the exarrow-rs concrete type from `ctx.connect_back()` would force every connect-back UDF to statically link exarrow-rs into its musl `.so`.

### Decision

`exasol-udf-sdk` defines the `ExaConnection` trait behind the `connect-back` feature. `exa-udf-runtime` provides the only implementation, backed by exarrow-rs. UDFs depend only on `exasol-udf-sdk` and `arrow`.

### Options Considered

| Option | Verdict |
|--------|---------|
| Trait in SDK, implementation in runtime | ✓ Chosen, the host already owns the connection infrastructure |
| Return `exarrow_rs::adbc::Connection` directly | ✗ Statically links exarrow-rs into every connect-back `.so` |

### Consequences

Without the `connect-back` feature, UDFs have no dependency on exarrow-rs or tokio. New connect-back methods need changes in both the SDK trait and the runtime.

## ADR: Dedicated OnceLock current_thread runtime for connect-back

**ID:** dedicated-oncelock-runtime-connect-back
**Plan:** `add-v2-rust-udf-complete`
**Status:** Accepted

### Context

Connect-back calls async exarrow-rs APIs from the synchronous ZMQ dispatch loop. The bridge must not restructure that loop.

### Decision

The runtime owns a `CONNECT_BACK_RT: OnceLock<tokio::runtime::Runtime>` (current_thread) and `block_on`s exarrow-rs async calls from the synchronous ZMQ dispatch loop.

### Options Considered

| Option | Verdict |
|--------|---------|
| OnceLock current_thread runtime, `block_on` at call site | ✓ Chosen, async stays contained |
| Async dispatch loop | ✗ Breaks the I/O-free state machine invariant |
| Multi-thread tokio runtime | ✗ Unneeded concurrency for sequential calls |

### Consequences

Connect-back queries cannot overlap, which the sequential dispatch loop allows.

## ADR: cargo-exaudf hides the musl target triple from authors

**ID:** cargo-exaudf-hides-musl-target-triple
**Plan:** `add-v2-rust-udf-complete`
**Status:** Accepted

### Context

Deployable Rust UDF artifacts must target `x86_64-unknown-linux-musl` for fully-static linking.

### Decision

`cargo exaudf build` always targets `x86_64-unknown-linux-musl`, runs `rustup target add` if the target is absent, and never exposes the triple to the author.

### Options Considered

| Option | Verdict |
|--------|---------|
| Hide the triple, auto-install via rustup | ✓ Chosen |
| Require `--target` | ✗ Exposes an implementation detail |
| Require pre-installed musl target | ✗ Breaks first run, unhelpful cargo errors |

### Consequences

Authors use only `cargo exaudf new/build/validate`. The author's host needs `rustup`.

## ADR: Connect-back uses named-connection metadata, not an internal proxy

**ID:** connect-back-named-connection-metadata
**Plan:** `add-v2-rust-udf-complete`
**Status:** Accepted

### Context

The reference SLC (`exasol/script-languages`) treats a named connection as a routable endpoint plus password, not as an internal proxy token. Pointing it at the container's loopback/eth0 `:8563` causes a SIGABRT on `2026.1.0`.

### Decision

The runtime opens the connect-back connection to the `address`/`user`/`password` returned by the on-demand `MT_IMPORT` (`PB_IMPORT_CONNECTION_INFORMATION`) response, connecting as an external client. There is no internal connect-back proxy endpoint.

### Options Considered

| Option | Verdict |
|--------|---------|
| Connect to `connection_information_rep.address` as an external client | ✓ Chosen, matches the reference SLC |
| Internal proxy at loopback/eth0 `:8563` | ✗ Causes the `2026.1.0` SIGABRT |

### Consequences

The `CB_SELF` test connection must be created `TO '<routable-endpoint>:8563'`, reachable from the UDF sandbox network namespace. `exa.get_connection(name)` passes the metadata to UDF code, which connects as an ordinary external client.

## ADR: Native binary protocol is the mandatory connect-back transport

**ID:** native-binary-protocol-connect-back-transport
**Plan:** `add-v2-rust-udf-complete`
**Status:** Accepted

### Context

exarrow-rs supports the `native` binary protocol (default) and `websocket` transports. The native protocol is faster and matches the main-session transport.

### Decision

The connect-back connection MUST use the exarrow-rs native binary protocol. The runtime builds the DSN with no `transport=` override and relies on the default `native` feature.

### Options Considered

| Option | Verdict |
|--------|---------|
| Native protocol, no `transport=` override | ✓ Chosen, faster, simpler DSN |
| `transport=websocket` | ✗ Pin was never a transport requirement |
| Benchmark native vs WebSocket | ✗ Decision already made |

### Consequences

The WebSocket connect-back path is untested and unsupported. A future DB version that breaks the native connect-back handshake requires re-evaluation.
