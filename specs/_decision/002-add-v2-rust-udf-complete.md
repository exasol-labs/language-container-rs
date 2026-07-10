# Decisions: add-v2-rust-udf-complete

## ADR: ExaConnection trait in the SDK, implemented by the runtime

**ID:** exaconnection-trait-in-sdk
**Plan:** `add-v2-rust-udf-complete`
**Status:** Accepted

### Context

Connect-back lets UDFs query the database mid-execution. The SDK design requires a connect-back API (`ExaConnection`, `exa()`, `exa_named()`, `exa_connect()`). A decision was needed on where to locate the trait and its concrete implementation — specifically whether to expose the exarrow-rs concrete type directly from `ctx.connect_back()` or to hide it behind a trait.

### Decision

Connect-back is exposed as an `ExaConnection` trait defined in `exasol-udf-sdk` (behind the `connect-back` feature). The `exa-udf-runtime` crate provides the only implementation, backed by exarrow-rs. UDFs depend only on `exasol-udf-sdk` + `arrow`.

### Options Considered

| Option | Verdict |
|--------|---------|
| ExaConnection trait in SDK, impl in runtime | ✓ Chosen — UDFs avoid statically linking exarrow-rs; the host process already owns the connection infrastructure (design §11.3) |
| Return `exarrow_rs::adbc::Connection` directly | ✗ Rejected — forces every connect-back UDF to statically link exarrow-rs into its musl `.so`, expensive and unnecessary |

### Consequences

UDFs have no compile-time dependency on exarrow-rs or tokio when the `connect-back` feature is absent. The runtime is the single owner of the exarrow-rs link. Adding new connect-back methods requires updating the trait in the SDK and the implementation in the runtime.

## ADR: Dedicated OnceLock current_thread runtime for connect-back

**ID:** dedicated-oncelock-runtime-connect-back
**Plan:** `add-v2-rust-udf-complete`
**Status:** Accepted

### Context

Connect-back requires calling async exarrow-rs APIs from inside the synchronous ZMQ dispatch loop. A decision was needed on how to bridge the sync/async boundary without restructuring the dispatch loop.

### Decision

The runtime owns a `CONNECT_BACK_RT: OnceLock<tokio::runtime::Runtime>` (current_thread) and `block_on`s exarrow-rs async calls from the synchronous ZMQ dispatch loop.

### Options Considered

| Option | Verdict |
|--------|---------|
| OnceLock current_thread runtime, block_on at call site | ✓ Chosen — ZMQ loop stays blocking; async is strictly contained; no cross-contamination with the protocol state machine |
| Make the whole dispatch loop async | ✗ Rejected — requires restructuring the I/O-free state machine invariant established in v1 |
| Spawn a multi-thread tokio runtime | ✗ Rejected — unnecessary concurrency for a sequentially-driven connect-back call; harder to reason about |

### Consequences

Async is strictly contained to connect-back calls. The ZMQ dispatch loop remains synchronous and I/O-free as designed. The current_thread runtime means connect-back queries cannot overlap; this is acceptable since the dispatch loop is sequential.

## ADR: JIT explicitly out of scope; compiler.rs returns UnsupportedFeature

**ID:** jit-out-of-scope-v2
**Plan:** `add-v2-rust-udf-complete`
**Status:** Accepted

### Context

The design document describes an Option C (JIT) compilation path alongside Option A (precompiled `.so`). v1 had already deferred JIT. A decision was needed on whether to implement JIT in v2 as part of completing the Rust SLC.

### Decision

Do not spec or implement JIT in v2. `compiler.rs` remains returning `UnsupportedFeature`.

### Options Considered

| Option | Verdict |
|--------|---------|
| Keep JIT out of scope | ✓ Chosen — keeps v2 focused on the four declared capability areas; avoids the ~1.4 GB jit container surface |
| Implement Option C in-container compilation in v2 | ✗ Rejected — deferred by user; not required for the connect-back, single-call, annotation, or CLI goals |

### Consequences

The slim image supports only `.so` artifacts uploaded to BucketFS. JIT/Option C must be added in a future plan. The `compiler.rs` entry point returns a clear unsupported error.

## ADR: cargo-exaudf hides the musl target triple from authors

**ID:** cargo-exaudf-hides-musl-target-triple
**Plan:** `add-v2-rust-udf-complete`
**Status:** Accepted

### Context

All deployable Rust UDF artifacts must target `x86_64-unknown-linux-musl` for fully-static linking. Authors must install this target via `rustup` before building. A decision was needed on whether to expose this detail or abstract it away in the CLI.

### Decision

`cargo exaudf build` always targets `x86_64-unknown-linux-musl`, auto-installing the target via `rustup target add` if absent, and never exposes the triple to the author.

### Options Considered

| Option | Verdict |
|--------|---------|
| Hide the triple; auto-install via rustup | ✓ Chosen — fully-static musl is the only supported deploy artifact; hiding the triple removes a class of author error; matches the mission's documented author workflow |
| Require authors to pass `--target` | ✗ Rejected — exposes an implementation detail authors should not need to know |
| Require authors to pre-install the musl target | ✗ Rejected — breaks first-run experience; error messages from cargo are unhelpful for newcomers |

### Consequences

Authors interact only with `cargo exaudf new/build/validate`. The musl target is an implementation detail of the CLI. The `rustup` binary must be available on the author's host.

## ADR: Connect-back uses named-connection metadata, not an internal proxy

**ID:** connect-back-named-connection-metadata
**Plan:** `add-v2-rust-udf-complete`
**Status:** Accepted

### Context

The connect-back mechanism lets UDFs connect to the database from inside the UDF sandbox. The v1 implementation treated the named connection as an internal proxy and pointed it at the container's own loopback/eth0 `:8563`, causing a SIGABRT on `2026.1.0`. Investigation of `exasol/script-languages` revealed the true mechanism.

### Decision

The runtime opens the connect-back connection to the `address`/`user`/`password` returned by the on-demand `MT_IMPORT` (`PB_IMPORT_CONNECTION_INFORMATION`) response, connecting exactly as an external client would. There is no dedicated internal connect-back proxy endpoint.

### Options Considered

| Option | Verdict |
|--------|---------|
| Connect to `connection_information_rep.address` as an external client | ✓ Chosen — matches the reference SLC (Python/Java); `CREATE CONNECTION ... TO '<address>'` is a routable endpoint + password, not a proxy token |
| Keep treating the named connection as an internal proxy at loopback/eth0 `:8563` | ✗ Rejected — caused the `2026.1.0` SIGABRT; contradicts how the reference SLC works |

### Consequences

The `CB_SELF` test connection must be created `TO '<routable-endpoint>:8563'` reachable from the UDF sandbox network namespace. The `exa.get_connection(name)` pattern passes metadata to UDF code, which connects using that metadata as an ordinary external client.

## ADR: Native binary protocol is the mandatory connect-back transport

**ID:** native-binary-protocol-connect-back-transport
**Plan:** `add-v2-rust-udf-complete`
**Status:** Accepted

### Context

Connect-back opens a new connection from inside the UDF sandbox back to a routable Exasol endpoint. exarrow-rs supports two transports: `native` (binary protocol, the default) and `websocket`. The v1 code hard-pinned `transport=websocket`. Task 6.2 was originally an open "empirically compare and choose" question.

### Decision

The connect-back connection MUST use the exarrow-rs native binary protocol. The runtime achieves this by building the DSN with no `transport=` override, relying on exarrow-rs's default `native` feature. The `transport=websocket` pin is removed.

### Options Considered

| Option | Verdict |
|--------|---------|
| Native binary protocol (no transport= override) | ✓ Chosen — faster than WebSocket; matches the main-session transport; simpler DSN; user mandated it |
| Keep `transport=websocket` | ✗ Rejected — was only assumed necessary due to the address-misuse SIGABRT (decision ADR-012), not a transport requirement |
| Empirically benchmark native vs WebSocket | ✗ Rejected — user made the call; an open comparison is unnecessary |

### Consequences

The WebSocket connect-back path is left untested and unsupported. `transport=websocket` is no longer emitted in the DSN. If a future Exasol DB version rejects or breaks the native connect-back handshake, this decision must be re-evaluated.
