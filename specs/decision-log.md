# Architecture Decision Records

<!-- ADRs are numbered sequentially starting from ADR-001. Never renumber. -->
<!-- recorder-agent appends new ADRs from plan decision logs. -->

---

## ADR-001: v1 uses Option A (precompiled .so) only; JIT returns unsupported

**Date:** 2026-06-05
**Plan:** `add-v1-rust-udf-slim`
**Status:** Accepted

### Context

The design document describes two execution paths: Option A (precompiled `.so` loaded at runtime via `dlopen`) and Option C (JIT compilation inside the language container). JIT requires a ~1.4 GB image with a vendored Cargo registry and an in-container compile/cache pipeline. A decision was needed on which to implement in v1 to prove the full protocol/SDK/loader/dispatch surface end-to-end.

### Decision

Implement only the precompiled `.so` execution path (Option A) in v1. The runtime's compiler entry point returns an unsupported-feature error for the JIT (Option C) path.

### Options Considered

| Option | Verdict |
|--------|---------|
| Option A only (precompiled .so) | ✓ Chosen — exercises the entire protocol, SDK, macro, loader, and dispatch surface with far less infrastructure; all v1 proof scenarios pass |
| Option C (JIT) also | ✗ Rejected — requires the ~1.4 GB image, vendored registry, and in-container pipeline; not needed to prove correctness end-to-end |

### Consequences

The slim image supports only `.so` artifacts uploaded to BucketFS. JIT/Option C must be added in a future plan. The `compiler.rs` entry point returns a clear unsupported error, making the limitation explicit rather than silent.

---

## ADR-002: Connect-back excluded from v1; connect-back Cargo feature is a no-op

**Date:** 2026-06-05
**Plan:** `add-v1-rust-udf-slim`
**Status:** Accepted

### Context

The SDK design includes a connect-back API (`ExaConnection`, `exa()`, `exa_named()`, `exa_connect()`) that lets UDFs query the database mid-execution. This requires `tokio`, `exarrow-rs` on the UDF/runtime side, and a credential-resolution path for `PB_IMPORT_CONNECTION_INFORMATION` and named `CONNECTION` objects. None of the three v1 proof scenarios (scalar doubler, set filter, JSON parse) need connect-back.

### Decision

Do not implement connect-back in v1. The `connect-back` Cargo feature is declared in the SDK but compiles to nothing.

### Options Considered

| Option | Verdict |
|--------|---------|
| Exclude connect-back from v1 | ✓ Chosen — shrinks v1 surface, avoids async/sync duality risk, not needed for proof scenarios |
| Wire connect-back feature now | ✗ Rejected — pulls in `tokio` + `exarrow-rs` on the UDF side and a whole credential path that none of the v1 scenarios require |

### Consequences

UDF authors cannot call `exa()` in v1. The feature stub ensures the SDK API surface is declared but guards against accidental use at compile time. Connect-back must be implemented in a future plan.

---

## ADR-003: Integration tests use testcontainers-rs with a pinned DB image in privileged mode

**Date:** 2026-06-05
**Plan:** `add-v1-rust-udf-slim`
**Status:** Accepted

### Context

The integration tests must prove the full BucketFS upload + `ALTER SESSION` + `CREATE SCRIPT` + `SELECT` path against a real Exasol database. Options included a manual docker-compose harness, the script-languages emulator, or an automated testcontainers approach.

### Decision

Use `testcontainers-rs` to start `exasol/docker-db:2026.1.0` with `with_privileged(true)`, exposing DB port `8563` and BucketFS port `2580`. Tests are gated behind an `integration` Cargo feature.

### Options Considered

| Option | Verdict |
|--------|---------|
| testcontainers-rs, pinned image, privileged | ✓ Chosen — self-contained, CI-friendly, RAII teardown, proves the real BucketFS + SQL path |
| Manual docker-compose harness | ✗ Rejected — brittle lifecycle management, harder to gate in CI |
| Script-languages emulator | ✗ Rejected — cannot prove BucketFS upload, `ALTER SESSION`, `CREATE SCRIPT`, or `SELECT` — exactly what v1 must demonstrate |

### Consequences

Docker must be available with privileged-container support in any environment running the integration tests. The `integration` feature gate keeps the default `cargo test` fast and Docker-free.

---

## ADR-004: BucketFS upload via HTTP PUT and SQL via exarrow-rs directly

**Date:** 2026-06-05
**Plan:** `add-v1-rust-udf-slim`
**Status:** Accepted

### Context

The integration harness needs to upload `.so` artifacts to BucketFS and execute SQL assertions. Two approaches were considered: shelling out to `exapump` (a CLI), or calling APIs directly from the test crate.

### Decision

Upload BucketFS artifacts with `reqwest` HTTP PUT to `http://w:<write-password>@<host>:<bucketfs-port>/<bucket>/<path>` and run all SQL through `exarrow-rs` with `validate_server_certificate(false)`.

### Options Considered

| Option | Verdict |
|--------|---------|
| HTTP PUT (reqwest) + exarrow-rs directly | ✓ Chosen — both are directly callable from the test crate; no process dependency; matches project's exarrow-rs preference |
| Shell out to exapump | ✗ Rejected — exapump is a CLI, not a library; shelling out adds a process dependency and brittle output parsing |

### Consequences

The `it` crate takes `reqwest` and `exarrow-rs` as dev-dependencies. SSL certificate validation is disabled per project rules (`validateservercertificate=0`). `exapump` is still used outside the test harness for other DBA operations.

---

## ADR-005: Pure I/O-free protocol state machine separated from ZMQ transport

**Date:** 2026-06-05
**Plan:** `add-v1-rust-udf-slim`
**Status:** Accepted

### Context

The protocol state machine is the subtlest part of the system — it must handle more than a dozen message types and phase transitions correctly. A decision was needed on whether to fold socket I/O directly into the state machine or separate concerns.

### Decision

`exa-zmq-protocol::Protocol` consumes decoded `ExascriptResponse` values and produces `ExascriptRequest`/`HostEvent` values with no socket I/O. The DEALER socket lives only in `ZmqTransport`.

### Options Considered

| Option | Verdict |
|--------|---------|
| Pure state machine, I/O in ZmqTransport | ✓ Chosen — entire message-ordering logic is unit-testable with fixtures; isolates the one piece that must talk to libzmq |
| Fold socket I/O into the state machine | ✗ Rejected — makes the most complex and error-prone logic impossible to unit-test deterministically |

### Consequences

The state machine can be driven entirely from fixture-based unit tests, covering all phase transitions and edge cases without a real ZMQ socket. `ZmqTransport` is the only piece requiring integration-level tests.

---

## ADR-006: Single C-ABI crossing with ABI-version and fingerprint gating

**Date:** 2026-06-05
**Plan:** `add-v1-rust-udf-slim`
**Status:** Accepted

### Context

Rust has no stable ABI. Passing rich trait objects across a `dlopen` boundary is undefined-behavior-prone. A design was needed for how the host runtime loads and calls UDF code at runtime, and how to catch toolchain mismatches before they cause UB.

### Decision

The only FFI boundary is `extern "C" fn __exa_udf_entry() -> *const ExaUdfVTable`. The loader checks `abi_version == 1` and the `sdk_fingerprint` before calling `create`. The `#[exasol_udf]` macro embeds a `build.rs`-baked fingerprint and wraps `run` in `catch_unwind`.

### Options Considered

| Option | Verdict |
|--------|---------|
| Single #[repr(C)] vtable + abi_version + fingerprint | ✓ Chosen — turns toolchain mismatch into a clear error; catch_unwind prevents FFI unwind UB; rich traits stay host-side |
| Pass rich trait objects across the boundary | ✗ Rejected — Rust has no stable ABI; trait objects across dlopen are UB-prone |
| Skip fingerprint check | ✗ Rejected — silent UB on toolchain mismatch is worse than a clear rejection error |

### Consequences

A `.so` built with a mismatched toolchain or SDK version is rejected at load time with a diagnostic error. Panic in user UDF code is caught and converted to an error code rather than unwinding across FFI. Rich trait objects and generics remain host-side only.

---

## ADR-007: Toolchain pinned to stable-1.84; exarrow-rs patched via [patch.crates-io] from the start

**Date:** 2026-06-05
**Plan:** `add-v1-rust-udf-slim`
**Status:** Accepted

### Context

The slim Docker image is built `FROM rust:1.84-bookworm`. The `EXA_SDK_FINGERPRINT` embeds a rustc hash, so it must be deterministic with respect to the container toolchain. The `exarrow-rs` crate lives at a local path and is not on crates.io; the IT harness and future connect-back phases need it as a shared dependency with deduplicated `arrow = "58"`.

### Decision

`rust-toolchain.toml` pins `channel = "1.84"` (with `targets = ["x86_64-unknown-linux-musl"]`). The root `Cargo.toml` includes `[patch.crates-io]` pointing `exarrow-rs` to `/home/talos/code/exarrow-rs` even though connect-back is out of v1.

### Options Considered

| Option | Verdict |
|--------|---------|
| Pin to 1.84; patch exarrow-rs now | ✓ Chosen — fingerprint is deterministic against the container toolchain; manifest stable for future phases; IT harness gets shared arrow = "58" |
| Float toolchain; add exarrow-rs patch later | ✗ Rejected — floating toolchain breaks fingerprint determinism; adding the patch later risks arrow deduplication conflicts |

### Consequences

All workspace crates build on Rust 1.84. The `it` crate has a `rust-version = "1.85"` due to a transitive dependency (`getrandom v0.4.2`), requiring integration tests to run with a separately installed toolchain (`cargo +1.91 test`). The musl `.so` artifacts use a custom target spec because Rust 1.84's built-in musl target has `dynamic-linking=false`.

---

## ADR-008: ExaConnection trait in the SDK, implemented by the runtime

**Date:** 2026-06-05
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

---

## ADR-009: Dedicated OnceLock current_thread runtime for connect-back

**Date:** 2026-06-05
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

---

## ADR-010: JIT explicitly out of scope; compiler.rs returns UnsupportedFeature

**Date:** 2026-06-05
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

---

## ADR-011: cargo-exaudf hides the musl target triple from authors

**Date:** 2026-06-05
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

---

## ADR-012: Connect-back uses named-connection metadata, not an internal proxy

**Date:** 2026-06-05
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

---

## ADR-013: Native binary protocol is the mandatory connect-back transport

**Date:** 2026-06-05
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

---

## ADR-014: Connect-back is always a new external-client session and a new transaction

**Date:** 2026-06-06
**Plan:** `fix-connect-back-external-client`
**Status:** Accepted

### Context

Connect-back lets UDFs query the database from inside `run()`. A question arose about whether the runtime should attempt to share the invoking query's session or transaction, or open an independent external-client connection.

### Decision

The runtime opens connect-back as an ordinary external-client login to the `address`/`user`/`password` returned by `MT_IMPORT` (`PB_IMPORT_CONNECTION_INFORMATION`), establishing a new session and a new transaction. This is the same pattern as PyExasol's `exa.get_connection(NAME)` followed by an independent connect.

### Options Considered

| Option | Verdict |
|--------|---------|
| New external-client session, new transaction | ✓ Chosen — matches reference SLCs (Python/Java/strata-rs); core cannot share a UDF transaction anyway |
| Share/join the invoking query's session or transaction | ✗ Rejected — Exasol core cannot share the invoking query's transaction with a container UDF; the internal-proxy path at loopback/eth0 `:8563` caused the original SIGABRT |

### Consequences

The `CB_SELF` named connection must be created `TO '<routable-endpoint>:8563'` reachable from the UDF sandbox network namespace. Connect-back queries run in a separate transaction and do not see the caller's uncommitted state. Operators configure the endpoint; the UDF artifact stays generic via `%connection <NAME>`.

---

## ADR-015: Docker-host-gateway address does not resolve the 2026.latest SIGABRT

**Date:** 2026-06-06
**Plan:** `fix-connect-back-external-client`
**Status:** Accepted

### Context

Commit `7de7357` changed the connect-back address to the Docker host gateway (instead of the container's loopback/eth0), hypothesising this would let the connect-back act as an external client and avoid the server-side SIGABRT. This plan ran a fresh integration suite on `2026-06-06` to verify the hypothesis.

### Decision

Record empirically that the SIGABRT persists on `exasol/docker-db:2026.latest` (image id `b81d80f63d10`, identical to `2026.1.0`) even with the Docker gateway external-client address. The crash is server-side, signal 6, and triggered by the core spawning a connect-back session for any container UDF — independent of address or transport. The SLC implementation is correct; the blocker is an upstream core defect.

Evidence from the `2026-06-06` run:
- 6 / 8 scenarios PASS (scalar, set, json, udf-error, both single-call).
- Both connect-back scenarios FAIL with `peer closed connection without sending TLS close_notify` on the outer session.
- Container log: `child <pid> (Part:40 Node:0 exasql) terminated with signal 6. (core dumped)` immediately after `Part:44` (connect-back session process) is spawned.

### Options Considered

| Option | Verdict |
|--------|---------|
| Record crash as unresolved upstream blocker; keep scenarios as known-failing gates | ✓ Chosen — honest evidence; scenarios auto-turn-green on a patched image |
| Assume the gateway fix resolved it (prior hypothesis) | ✗ Rejected — direct re-verification contradicts the hypothesis |
| Delete connect-back scenarios | ✗ Rejected — they form a regression net for when a patched image ships |

### Consequences

Connect-back integration scenarios remain known-failing on `2026.latest`. No workaround exists within the SLC. The test suite dumps SIGABRT diagnostics on failure. Once Exasol ships a patched image, the scenarios should pass without any SLC code changes.

---

## ADR-016: ConnectionObject is a public SDK type; ConnInfo stays internal to the protocol layer

**Date:** 2026-06-07
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

---

## ADR-017: cluster_ip() returns the raw node IP with no port appended

**Date:** 2026-06-07
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

---

## ADR-018: connection(name) performs an on-demand MT_IMPORT during the blocked dispatch loop

**Date:** 2026-06-07
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

---

## ADR-020: Alpine image — build the client binary for x86_64-unknown-linux-musl

**Date:** 2026-06-08
**Plan:** `change-docker-alpine-base`
**Status:** Accepted

### Context

The original Alpine image design compiled `exaudfclient` for the musl target (`x86_64-unknown-linux-musl`) using a `rust:alpine` builder, aligning with the already-musl UDF `.so` artifacts. During implementation, two blockers ruled this out: Rust 1.96+ compiled binaries crashed in the Exasol UDF sandbox due to seccomp/CPU-instruction incompatibility, and the `exaudfclient` binary is executed directly on the glibc Debian Exasol host after BucketFS extraction — a musl binary would be ABI-incompatible there. The adopted approach bundled glibc runtime libs into the Alpine image instead. The decision entry records what was resolved at plan time; the implementation pivot is documented in the plan's spike notes.

### Decision

The Alpine builder stage compiles `exaudfclient` for `x86_64-unknown-linux-musl` on a `rust:alpine` builder, and the resulting musl binary is placed in the `alpine:3` runtime stage.

### Options Considered

| Option | Verdict |
|--------|---------|
| Compile for `x86_64-unknown-linux-musl` on `rust:alpine` | ✓ Chosen — aligns with already-musl UDF artifacts; Alpine is musl-based; no glibc compat shim needed |
| Keep a glibc binary and run it on Alpine via `gcompat` | ✗ Rejected — fragile and counter to the smaller-image goal |

### Consequences

The Alpine builder must install `zeromq-dev`, `protobuf-dev`, `pkgconfig`, and `musl-dev` via `apk`. The runtime binary requires no glibc loader on `alpine:3`. See plan spike notes for the implementation pivot to glibc-bundling that superseded this in practice.

---

## ADR-021: Alpine runtime uses LANG=C.UTF-8 instead of locale-gen

**Date:** 2026-06-08
**Plan:** `change-docker-alpine-base`
**Status:** Accepted

### Context

The Debian slim image runs `locale-gen en_US.UTF-8` to configure the locale. Alpine/musl ships no `locales` package and no `locale-gen` binary. A decision was needed on how to configure UTF-8 locale in the Alpine runtime stage.

### Decision

Set `ENV LANG=C.UTF-8` in the Alpine runtime stage. No locale package is installed; no `locale-gen` is run.

### Options Considered

| Option | Verdict |
|--------|---------|
| `ENV LANG=C.UTF-8`, no locale package | ✓ Chosen — `C.UTF-8` is the musl default and sufficient for UDF text handling; keeps the image minimal; `locale-gen` does not exist on Alpine |
| Install `musl-locales` and generate `en_US.UTF-8` | ✗ Rejected — unnecessary weight; matches the Debian convention but adds extra packages without benefit |

### Consequences

The Alpine runtime carries no locale package. `C.UTF-8` provides UTF-8 string semantics adequate for UDF text handling. The absence of `locale-gen` is a non-issue on Alpine/musl. The runtime stage installs only `ca-certificates` via `apk`.

---

## ADR-019: Switch the client ZMQ transport from DEALER to REQ to match the database's REP socket

**Date:** 2026-06-08
**Plan:** `fix-zmq-req-socket`
**Status:** Accepted

### Context

The UDF client transport (`ZmqTransport`) was opening a `DEALER` socket and manually inserting an empty delimiter frame on `send` and discarding one on `recv` to imitate the `DEALER`/`ROUTER` multi-frame envelope. The Exasol architect confirmed the database actually binds a `REP` socket — not `ROUTER` — and this was validated against the Python3 SLC reference implementation (`exasol/script-languages-release`). A `REP` peer enforces strict request/reply alternation and delivers/expects exactly one payload frame; it does not carry routing identities or speak the `DEALER`/`ROUTER` multi-frame envelope. The `DEALER` client was using the wrong wire shape and relied on asynchronous send/recv semantics the DB does not support.

### Decision

Use `zmq::REQ` in `ZmqTransport::connect`. Let the `REQ` socket manage the request/reply delimiter automatically: `send` writes a single payload frame, `recv` reads a single payload frame. The DB's `REP` socket mirrors this exactly.

### Options Considered

| Option | Verdict |
|--------|---------|
| Use `zmq::REQ` (canonical `REP` counterpart) | ✓ Chosen — `REP` peers reject the `DEALER` envelope shape; `REQ` is the canonical, lock-step counterpart and removes manual framing bugs. Confirmed against the Python3 SLC reference. |
| Keep `DEALER` with manual empty-delimiter framing | ✗ Rejected — `REP` does not speak the `DEALER`/`ROUTER` multi-frame envelope; the manual delimiter insertion was the root cause of the post-`MT_CLIENT` hang |
| Use `DEALER` with an explicit delimiter sent to a `REP` peer | ✗ Rejected — fragile and non-idiomatic; `REQ` is the canonical counterpart and removes all hand-rolled framing |

### Consequences

The `send` implementation no longer prepends an empty delimiter frame; the `recv` implementation no longer discards one. The `REQ` socket enforces lock-step alternation that the protocol state machine already assumes. The transport integration tests now mock the DB with a `zmq::REP` peer, matching the real wire shape. The end-to-end `db_roundtrip` integration test (gated on Docker) exercises the full `REQ`/`REP` exchange against the live DB.

---

## ADR-022: Connect-back targets the node's own SQL endpoint over TCP

**Date:** 2026-06-09
**Plan:** `fix-connect-back-version-matrix`
**Status:** Accepted

### Context

The earlier connect-back implementation created `CB_SELF TO '<docker-host-gateway>:<mapped-port>'` and routed the connect-back `exarrow-rs` session through Docker NAT. This path caused the parent SQL session to terminate with signal 6. The root cause was the routing choice, not a database defect. Supersedes the framing of ADR-015.

### Decision

`CB_SELF` is created `TO '<connect_back_sql_address()>'`. The harness selects the address per deployment mode: in testcontainers mode it returns `<container-eth0-ip>:8563` (the container's own `eth0` address, bypassing NAT); in external mode (`EXASOL_HOST` set) it returns `<host>:<db_port>` (the cluster's routable SQL endpoint the harness already carries). The connect-back `exarrow-rs` session connects over plain TCP as a regular external client. The query and DML scenarios become hard assertions on every version.

### Options Considered

| Option | Verdict |
|--------|---------|
| Deployment-mode-aware address via `Harness::connect_back_sql_address()` | ✓ Chosen — direct TCP to the node's own SQL endpoint is the supported client path; mode distinction is essential because `container_inner_ip()` is Docker-only |
| Docker host gateway + host-mapped port | ✗ Rejected — the NAT path that caused the original SIGABRT |
| Hard-code `container_inner_ip():8563` for all modes | ✗ Rejected — `container_inner_ip()` requires `docker exec`; fails on real non-Docker clusters |
| Container loopback / internal-proxy framing | ✗ Rejected — caused the original SIGABRT in ADR-015 |

### Consequences

Connect-back query and DML scenarios pass as hard assertions on all three versions in the matrix (`2025.1`, `2025.2`, `2026.1`). `container_connect_back_address()` and the Docker-gateway address helper are removed as dead code. ADR-015 is superseded.

---

## ADR-023: The UDF↔DB ZMQ transport cannot be forced to TCP via SCRIPT_LANGUAGES

**Date:** 2026-06-09
**Plan:** `fix-connect-back-version-matrix`
**Status:** Accepted

### Context

An interview request asked to move `cluster_ip()` to always receive a `tcp://` ZMQ endpoint by changing the `SCRIPT_LANGUAGES` string. Investigation of `exaudflib_main.cc` showed this is not possible: `argv[1]`'s transport scheme is chosen by the database at launch, not by `SCRIPT_LANGUAGES`. On single-node `exasol/docker-db`, the database always passes `ipc://` for a locally-launched (`localzmq`) container.

### Decision

Do not attempt to change the `localzmq` transport prefix in `SCRIPT_LANGUAGES`. The ZMQ endpoint transport is a database-side concern. For `cluster_ip()`, the solution (reading the network interface instead of parsing the endpoint) is captured in ADR-025. The premise correction — that the transport cannot be forced — stands independently.

### Options Considered

| Option | Verdict |
|--------|---------|
| Accept that ZMQ transport is DB-controlled; address `cluster_ip()` separately | ✓ Chosen — correct description of the constraint; avoids impossible workarounds |
| Swap `localzmq` for a TCP transport prefix in `SCRIPT_LANGUAGES` | ✗ Rejected — `argv[1]`'s scheme is chosen by the DB at launch; `SCRIPT_LANGUAGES` has no flag to flip this |
| Use a `tcp:` `argv[1]` to select remote-client mode | ✗ Rejected — remote-client mode is a deployment model `exasol/docker-db` single-node does not use |

### Consequences

The ZMQ socket transport (IPC on single-node Docker, TCP on multi-node clusters) remains DB-controlled and opaque to the SLC. No code or configuration change can force a TCP ZMQ endpoint on single-node Docker. The `cluster_ip()` fix (ADR-025) does not depend on the ZMQ transport at all.

---

## ADR-024: Cargo features declare supported versions; runtime env var selects the active one

**Date:** 2026-06-09
**Plan:** `fix-connect-back-version-matrix`
**Status:** Accepted

### Context

The CI matrix runs three Exasol versions (`2025.1`, `2025.2`, `2026.1`). The `build-artifacts` job compiles `it-runner` once and every matrix job reuses that single binary. Cargo features are compile-time, so a reused binary cannot have a different feature set per matrix entry.

### Decision

Add `db-2025-1`, `db-2025-2`, `db-2026-1` features to `crates/it/Cargo.toml` with `default = ["db-2026-1"]`. These features are capability declarations only — no `cfg`-gated test bodies. Actual per-version branching (image tag selection) happens at runtime via `EXASOL_DB_SERIES`, falling back to the compiled default when unset. Unknown values are rejected with a clear error.

### Options Considered

| Option | Verdict |
|--------|---------|
| Compile-time features as declarations; runtime `EXASOL_DB_SERIES` for selection | ✓ Chosen — single artifact; honours "Cargo feature per version" intent; runtime env is flexible per matrix entry |
| Compile one `it-runner` per version (matrix in `build-artifacts`) | ✗ Rejected — triples build time and cache size for no behavioural gain |
| Use `cfg`-gated test bodies per feature | ✗ Rejected — binary compiled once cannot carry per-matrix `cfg` |

### Consequences

`build-artifacts` compiles `it-runner` once with `--features integration,db-2026-1`. Every matrix job sets `EXASOL_DB_SERIES` to select version behaviour at runtime. Local `cargo test` with no env var runs the `2026-1` series (the default). Unrecognised values fail fast.

---

## ADR-025: cluster_ip() reads the node IP from the network interface instead of parsing the ZMQ endpoint

**Date:** 2026-06-09
**Plan:** `fix-connect-back-version-matrix`
**Status:** Accepted

### Context

The original `cluster_ip()` parsed the node IP out of the ZMQ endpoint string (`argv[1]`). On single-node `exasol/docker-db` the database passes `ipc://` with no node IP to parse, so `cluster_ip()` returned an error there. ADR-023 establishes that the transport cannot be forced to TCP. A different implementation strategy was needed.

### Decision

`cluster_ip()` (in `crates/exa-udf-runtime/src/rowset.rs`) reads the local node's primary IPv4 from the network interface — the first non-loopback IPv4 of the UDF process (e.g. container `eth0`) — via `libc::getifaddrs`, instead of parsing the ZMQ endpoint string. `parse_cluster_ip()` in `crates/exa-udf-runtime/src/artifact.rs` is removed as dead code. The `connect_back_cluster_ip_emits_node_ip` scenario becomes a hard IPv4 assertion on every series.

### Options Considered

| Option | Verdict |
|--------|---------|
| Read primary IPv4 from network interface via `libc::getifaddrs` | ✓ Chosen — works identically on single-node Docker and multi-node TCP clusters; `libc` is already a workspace dependency; collapses to one hard assertion |
| Parse IP from ZMQ endpoint string | ✗ Rejected — fails on single-node Docker because the DB passes `ipc://` with no IP |
| Force TCP ZMQ transport via `SCRIPT_LANGUAGES` | ✗ Rejected — infeasible; see ADR-023 |
| Assert two branches with runtime severity flag (`EXASOL_DB_SERIES`) | ✗ Rejected — superseded by this approach; reading the interface eliminates the topology-dependent branch |
| Unconditionally skip `cluster_ip()` on Docker | ✗ Rejected — removes test coverage on the most common development environment |

### Consequences

`cluster_ip()` returns a valid IPv4 on both single-node Docker and multi-node TCP deployments. `parse_cluster_ip()` is dead code and is removed. The `connect_back_cluster_ip_emits_node_ip` integration scenario has no severity branch and no unconditional skip — it is a hard assertion on every version in the matrix. The `EXASOL_DB_SERIES` flag remains available for other future version-specific behaviour but no longer gates `cluster_ip`.

---

## ADR-026: CB_SELF address is deployment-mode-aware via Harness::connect_back_sql_address()

**Date:** 2026-06-09
**Plan:** `fix-connect-back-version-matrix`
**Status:** Accepted

### Context

The `CB_SELF` address must be a direct TCP path to the node's SQL endpoint reachable from the UDF sandbox. In testcontainers mode the harness `host:db_port` is the NAT-mapped ephemeral host port (the original crashing path), so the container's `eth0` address must be used instead. In external mode (a real cluster, `EXASOL_HOST` set) there is no container to `docker exec` into, so `container_inner_ip()` would fail and `host:db_port` is the correct address.

### Decision

Add `Harness::connect_back_sql_address()` to `crates/it/src/lib.rs`. In testcontainers mode (`self._container.is_some()`) it returns `format!("{}:8563", self.container_inner_ip().await?)`. In external mode (`self._container.is_none()`) it returns `format!("{}:{}", self.host, self.db_port)`. No new env var is introduced. `container_connect_back_address()` is removed as dead code.

### Options Considered

| Option | Verdict |
|--------|---------|
| Mode-aware `connect_back_sql_address()` branching on `self._container.is_some()` | ✓ Chosen — reuses state the `Harness` already carries; correct for both local Docker and real clusters; no new env var |
| Hard-code `container_inner_ip():8563` for all modes | ✗ Rejected — `container_inner_ip()` requires `docker exec`; fails on real non-Docker clusters |
| Reuse `host:db_port` for all modes | ✗ Rejected — in testcontainers mode `host:db_port` is the NAT-mapped ephemeral port (the original crashing path) |

### Consequences

`CB_SELF` is always a direct TCP path to the node's SQL endpoint regardless of deployment mode. `container_connect_back_address()` is removed. The mode distinction is transparent to test scenarios — they call `connect_back_sql_address()` uniformly.
