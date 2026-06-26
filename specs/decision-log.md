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

---

## ADR-027: ABI version bump 2→3 for virtual_schema_adapter_call signature change

**Date:** 2026-06-11
**Plan:** `2026-06-11-vs-adapter-and-single-call-connect-back`
**Status:** Accepted

### Context

The `virtual_schema_adapter_call` vtable slot changes from a 2-argument signature `(json_arg, result)` to a 3-argument signature `(ctx, json_arg, result)` so VS adapters can receive the host `UdfContext` pointer and call `ctx.connection(...)` / `ctx.connect_back(...)` mid single-call. Any `.so` compiled against ABI v2 that happened to wire this slot would be called with an extra argument, producing undefined behavior. A decision was needed on whether to increment the ABI version or add a parallel slot.

### Decision

Increment `EXA_UDF_ABI_VERSION` from 2 to 3. The `virtual_schema_adapter_call` vtable slot uses the new 3-argument signature exclusively. The loader rejects any `.so` whose `abi_version` field does not equal 3 with a clear version-mismatch error.

### Options Considered

| Option | Verdict |
|--------|---------|
| Increment ABI version 2→3 | ✓ Chosen — slot signature change is a binary incompatibility; incrementing forces the loader to reject old `.so` files with a clear error rather than silently invoking the wrong signature |
| Keep ABI v2, add a separate parallel slot | ✗ Rejected — bloats the vtable and complicates dispatch for no gain; does not eliminate the incompatibility for any `.so` that wired the old slot |
| Struct-based calling convention | ✗ Rejected — adds complexity without addressing the root issue; the double-indirection ABI already proven by the `run` shim is sufficient |

### Consequences

All user `.so` artifacts compiled against ABI v2 must be recompiled. The loader will reject v2 artifacts with a clear version-mismatch error at load time. This dominates the semver bump to 0.5.0 under pre-1.0 rules.

---

## ADR-028: Row-major type-block packing with NULL cells skipping the type block

**Date:** 2026-06-11
**Plan:** `2026-06-11-vs-adapter-and-single-call-connect-back`
**Status:** Accepted

### Context

The prior `EmitBuffer::to_proto` / `InputRowSet::from_proto` implementation used column-major packing with `n_rows` placeholder entries per column in each type block, including placeholders for NULL cells. This produced silently wrong values when NULL cells appeared: a NULL in column 2 would still write a placeholder into the string block, causing all subsequent string values for that row to land in the wrong column position. The Exasol wire format is row-major with no NULL slots.

### Decision

Switch `EmitBuffer::to_proto` and `InputRowSet::from_proto` to row-major ordering within each type block (row then column). NULL cells do not push any placeholder slot into the type block — only the null-bitmap is updated. Per-type running cursors advance only on non-null cells. Output values are packed by declared column `ExaType`, not by runtime `Value` variant (e.g. a `Value::Int64` in a `Numeric` column is stringified into the string block).

### Options Considered

| Option | Verdict |
|--------|---------|
| Row-major packing, NULL cells skip type-block slot | ✓ Chosen — matches the confirmed C++ reference behavior; eliminates the silent column-value corruption; per-type cursors correctly handle mixed-NULL rows |
| Column-major with `n_rows` placeholder entries per column | ✗ Rejected — produced silently wrong values when NULLs appeared; confirmed to be incorrect |
| Row-major with NULL placeholders | ✗ Rejected — still produces wrong values; the placeholder is the root cause |

### Consequences

All output type blocks are now row-major. NULL cells occupy no slot in their type block — only the null-bitmap is set. The `push_placeholder` function is removed as dead code. This is a correctness fix; the wire format now matches the Exasol reference implementation.

---

## ADR-029: External-mode connect-back uses the container eth0 IP; one root cause, not two

**Date:** 2026-06-12
**Plan:** `fix-it-matrix-connect-back-address`
**Status:** Accepted

### Context

The new exarrow-style CI pipeline (PR #1, merged) failed the `integration` job on all three DB versions (`2025.1.11`, `2025.2.1`, `2026.1.0`), blocking the `release` job. The failing CI run (27425472641) showed two integration scenarios failing:

```
scenario python3_connect_back FAILED: ... VM crashed (Session: 1867805944649744384)
Error: query: SELECT TO_CHAR(double_it(21)) ... VM crashed (Session: 1867805944649744384)
```

Both failures carried the **same DB Session ID**, proving they hit one shared, poisoned connection rather than two independent bugs. The root cause was `connect_back_sql_address()` returning `localhost:8563` in external mode (`EXASOL_HOST=localhost`), which resolves to `127.0.0.1` — Exasol's internal CoreDB proxy. The proxy links the connect-back session to the invoking SQL worker (Part:40), triggering a VM SIGABRT. Because all scenarios ran sequentially on one shared `Connection`, the crash from scenario 2 (`python3_connect_back`) poisoned the session and scenario 3 (`double_it` / `scalar_double`) then failed on the dead VM.

A prior plan (`fix-connect-back-version-matrix`, 2026-06-10) had assumed that external mode meant a remote cluster with no Docker container to exec into, so it returned `host:db_port`. In fact, the IT suite's external mode always targets a local Docker container named `exasol-db` in both CI and local-repro configurations — `container_inner_ip()` was already usable via `docker exec exasol-db`, making `localhost:8563` the uniquely wrong address.

The rowset row-major codec (`crates/exa-udf-runtime/src/rowset.rs`) was considered but deliberately left untouched: the evidence (shared Session ID, scenario-2-crashes-first) demonstrated the scalar path was collateral damage, not the root cause.

### Decision

`Harness::connect_back_sql_address()` always resolves the container `eth0` IP via `container_inner_ip()` and returns `<container-eth0-ip>:8563` in **both** testcontainers and external mode. It must never return a loopback address; if IP resolution fails it errors loudly rather than falling back to `localhost`. The Python3 connect-back diagnostic runs on a dedicated throwaway `harness.connect()` connection so that any VM crash from that diagnostic cannot poison the shared connection used by the asserted scenarios.

### Options Considered

| Option | Verdict |
|--------|---------|
| External mode uses `container_inner_ip()` (same as testcontainers) | ✓ Chosen — container is always `exasol-db` and exec-able in CI/local; loopback is the only failing address; simplest correct fix |
| Keep `host:db_port` in external mode; add `EXASOL_CB_ADDRESS` env override | ✗ Rejected — `host:db_port` is `localhost:8563` in CI, which is exactly the crashing address; env override deferred to a future genuine remote-cluster use case |
| Diagnostic on throwaway connection | ✓ Chosen — isolation is cleaner and removes the cascade that caused the misdiagnosis |
| Reconnect main `conn` after the diagnostic | ✗ Rejected — does not prevent poisoning if the crash happens mid-setup; throwaway isolation is structurally safer |
| Audit/rewrite the rowset scalar path | ✗ Rejected — evidence (shared Session ID, scenario order) shows scalar path is collateral, not cause; `double_it` passes on a healthy session |

### Consequences

`connect_back_sql_address()` is now mode-independent: both testcontainers and external mode use `<container-eth0-ip>:8563`. A genuine remote-cluster (non-Docker) external mode is out of scope; an explicit `EXASOL_CB_ADDRESS` environment-variable override would cover that future use case. The rowset codec was not changed. All 12 integration scenarios pass locally in external mode against `exasol/docker-db:2026.1.0`; the fix is version-independent and expected to green the full CI matrix.

---

## ADR-030: Numeric represented as a custom Decimal { unscaled: i128, scale: u8 } newtype

**Date:** 2026-06-14
**Plan:** `change-sdk-type-system`
**Status:** Accepted

### Context

`Value::Numeric` must carry a lossless decimal representation for Exasol's `DECIMAL(p,s)` type, which supports up to 36 significant digits. The `rust_decimal` crate uses a 96-bit mantissa, capping precision at ~28-29 significant digits. Parsing a proto wire string through `rust_decimal` would silently lose precision on values with 29–36 significant digits. The project also has a musl-static-link constraint favouring minimal dependencies and zero data transformation on the hot path.

### Decision

Add a zero-dependency `Decimal { unscaled: i128, scale: u8 }` newtype in `exasol-udf-sdk::value` with `TryFrom<&str>`, `TryFrom<f64>`, and a lossless `Display`. `Value::Numeric` carries this `Decimal`.

### Options Considered

| Option | Verdict |
|--------|---------|
| Custom `Decimal { i128, u8 }` newtype | ✓ Chosen — `i128` holds 38 digits, covering Exasol's full range; zero new dependencies; exact lossless round-trip; aligns with the musl-static-link and zero-transformation goals |
| `rust_decimal::Decimal` | ✗ Rejected — capped at ~28-29 significant digits (96-bit mantissa); would silently lose precision on Exasol DECIMAL values with 29–36 digits |

### Consequences

The SDK carries a bespoke decimal type. Authors who need arithmetic must either use the `unscaled`/`scale` fields directly or convert to a floating-point type (with the documented precision trade-off). The `rust_decimal` crate is not added to the workspace. Lossless wire round-trip is guaranteed for Exasol's full 36-digit DECIMAL range.

---

## ADR-031: Deduplicate ExaType by making exa-zmq-protocol depend on exasol-udf-sdk

**Date:** 2026-06-14
**Plan:** `change-sdk-type-system`
**Status:** Accepted

### Context

`ExaType` was duplicated verbatim in both `exasol-udf-sdk` and `exa-zmq-protocol`, allowing the two copies to drift. The SDK is the author-facing home of the type model. Two options existed: add a new `exa-types` leaf crate that both depend on, or add a direct dependency edge from the protocol crate to the SDK.

### Decision

The single canonical `ExaType` lives in `exasol-udf-sdk::value`. `exa-zmq-protocol` adds a dependency edge on `exasol-udf-sdk` and re-exports that enum. The dependency graph becomes `protocol → {exa-proto, exasol-udf-sdk}` and `runtime → {protocol, exasol-udf-sdk}` — no cycle.

### Options Considered

| Option | Verdict |
|--------|---------|
| Add dependency edge `exa-zmq-protocol → exasol-udf-sdk` | ✓ Chosen — one edge addition produces no cycle; far less churn than introducing, versioning, and publishing a new crate; the SDK is already the natural home of the author-facing type model |
| Extract a new `exa-types` leaf crate | ✗ Rejected — requires creating, versioning, and publishing a new crate; significantly more churn for the same outcome |

### Consequences

`exa-zmq-protocol` gains a compile-time dependency on `exasol-udf-sdk`. The duplicate `ExaType` enum in `exa-zmq-protocol/src/meta.rs` is deleted. All downstream code uses `exasol_udf_sdk::value::ExaType` as the single type. The dependency graph remains acyclic.

---

## ADR-032: Extended Exasol types are String-backed Value payloads but distinct ExaType variants

**Date:** 2026-06-14
**Plan:** `change-sdk-type-system`
**Status:** Accepted

### Context

Several Exasol SQL types (`TIMESTAMP WITH LOCAL TIME ZONE`, `INTERVAL YEAR TO MONTH`, `INTERVAL DAY TO SECOND`, `GEOMETRY`, `HASHTYPE`, `CHAR`) are transmitted over the wire as a proto `STRING` block. Fully typed representations (e.g. `chrono::DateTime<FixedOffset>` for `TimestampTz`, dedicated interval/geometry structs) would require non-trivial parsing and new dependencies. A decision was needed on whether to make these types fully typed `Value` payloads or to distinguish them only at the `ExaType` level.

### Decision

Distinguish extended types at the `ExaType` level (new variants: `Char { size }`, `Geometry`, `HashType`, `IntervalYearToMonth`, `IntervalDayToSecond`, `TimestampTz`) refined from `type_name` at `ColumnMeta` construction time. The `Value` payload for all these types remains the wire `String`. Only `Date`, `Timestamp`, and `Numeric` become fully typed (`NaiveDate`, `NaiveDateTime`, `Decimal`).

### Options Considered

| Option | Verdict |
|--------|---------|
| Distinct `ExaType` variants, `String` wire payload | ✓ Chosen — proto block is a string and does not change; Exasol timezone and interval semantics are complex; `ExaType` gives authors the SQL-type information without conversion cost; zero allocation on the hot path |
| Fully typed `chrono::DateTime<FixedOffset>` for TimestampTz and dedicated interval/geometry structs | ✗ Rejected — complex semantics; proto block stays string anyway; rarely arithmetic-driven inside a UDF; would add non-trivial parsing and potentially new dependencies |

### Consequences

Authors receive the SQL type distinction via `ColumnMeta::typ` (`ExaType` variant) but receive the raw wire string as the `Value` payload for extended types. Extended-type arithmetic (timezone conversion, interval math) must be handled by the author. The `ExaType` refinement happens once at `ColumnMeta::from_pb`, downstream code sees a rich `ExaType` with no repeated `type_name` inspection.

---

## ADR-033: Raise the workspace MSRV pin from 1.84 to 1.92

**Date:** 2026-06-15
**Plan:** `change-toolchain-edition-2024`
**Status:** Accepted

### Context

The workspace `rust-toolchain.toml` was pinned at `channel = "1.84"`. This was a false floor: `connect-back` feature users already required Rust >= 1.85 (arrow 58 transitively pulls in edition-2024 crates that Rust 1.84 cannot parse), so those crates were excluded from `default-members` and built explicitly with `cargo +1.91` overrides. The CI and Docker builder images always ran 1.91. Three toolchain versions were in play simultaneously — workspace pin (1.84), CI/builder (1.91), and the effective floor for all members (1.85+). Raising the pin to 1.92 unifies all three on one version. 1.92 is the minimum that clears the edition-2024 floor (edition 2024 stabilized in 1.85) and matches the `rust-version = "1.92"` floor on the strata-rs iceberg side. The whole stack is unreleased, so raising the floor carries no downstream cost.

### Decision

Bump `rust-toolchain.toml` `channel` from `"1.84"` to `"1.92"`. Collapse the `default-members` split: move `connect-back-query`, `connect-back-insert`, `connect-back-crunch`, `connect-back-cluster-ip`, and `spike-connect` into `default-members`. Drop all `cargo +1.91` / `cargo +1.92` override prefixes in CI and scripts. Remove the per-crate `rust-version = "1.85"` from `crates/it` — the toolchain pin becomes the single MSRV source of truth.

### Options Considered

| Option | Verdict |
|--------|---------|
| Raise pin to 1.92; unify all three toolchains | ✓ Chosen — eliminates the `default-members` split and all `cargo +N` overrides; 1.92 satisfies the edition-2024 floor and the strata-rs iceberg floor; no downstream cost for an unreleased stack |
| Keep pin at 1.84; retain `default-members` split and `cargo +N` overrides | ✗ Rejected — the 1.84 floor was already effectively broken; maintaining the three-toolchain split is dead weight |
| Delete the pin entirely (float to latest stable) | ✗ Rejected — a pin is still wanted so `rustup` selects 1.92 deterministically; the Docker `rm rust-toolchain.toml` fallback to the image toolchain remains a clean no-op |

### Consequences

All workspace members including the `connect-back` crates build under plain `cargo` with no `+N` overrides. The `default-members` exclusions for `connect-back-*` and `spike-connect` are removed. The only remaining exclusion from `default-members` is `crates/it`, which stays out because it requires a live Exasol Docker container — a runtime dependency, not a toolchain constraint.

---

## ADR-034: Migrate the whole workspace to edition 2024

**Date:** 2026-06-15
**Plan:** `change-toolchain-edition-2024`
**Status:** Accepted

### Context

All 19 workspace crates (`crates/*/` and `test-udfs/*/`) were at `edition = "2021"`. Once the MSRV pin is raised to 1.92 (ADR-033), edition 2024 is available across the whole workspace. The north-star goal is one toolchain and one edition so a developer runs plain `cargo build` / `cargo test` with zero `+N` flags and zero edition juggling. Mixed editions (some crates 2021, some 2024) leave exactly the complexity the toolchain unification is meant to eliminate.

### Decision

Bump `edition` from `"2021"` to `"2024"` in all 19 crate manifests. Run `cargo fix --edition` per crate under the 1.92 toolchain to apply mechanical migrations. The single non-trivial code change edition 2024 requires in this workspace is the proc-macro FFI attribute, captured in ADR-035.

### Options Considered

| Option | Verdict |
|--------|---------|
| Migrate all 19 crates to edition 2024 | ✓ Chosen — one toolchain + one edition eliminates dual-edition complexity; the whole stack is unreleased so there is no backward-compatibility cost; `cargo fix --edition` handles mechanical migrations |
| Keep mixed editions (2021 for core crates, 2024 for test-udfs only) | ✗ Rejected — defeats the one-edition DX goal; forces per-invocation edition awareness to persist |
| Pin at latest stable instead of 1.92 | ✗ Rejected — 1.92 is the justified minimum satisfying both the edition-2024 floor and the strata-rs iceberg floor; pinning higher buys nothing and drifts from the strata-rs side |

### Consequences

All 19 crate manifests declare `edition = "2024"`. The `rust-version = "1.85"` field on `crates/it` is removed; the toolchain pin is the single MSRV declaration. The success criterion is satisfied: a developer runs `cargo build` / `cargo test` with zero `+N` flags and zero edition juggling.

---

## ADR-035: Proc-macro emits #[unsafe(no_mangle)] for the UDF entry point

**Date:** 2026-06-15
**Plan:** `change-toolchain-edition-2024`
**Status:** Accepted

### Context

The `exasol-udf-macros` proc-macro emitted a bare `#[no_mangle]` for the generated `__exa_udf_entry` FFI entry point. Edition 2024 promotes `#[no_mangle]`, `#[export_name]`, and `#[link_section]` to unsafe attributes. Proc-macro-emitted tokens are interpreted in the call-site crate's edition: once any consuming crate (test-udf crates) becomes edition 2024, the bare `#[no_mangle]` errors at the call site, not in the macro crate. Gating the attribute on the call-site edition from inside a proc-macro is impossible — proc-macros cannot reliably read the consumer's edition.

### Decision

Change the `exasol-udf-macros` proc-macro to emit `#[unsafe(no_mangle)]` instead of bare `#[no_mangle]` for the generated `__exa_udf_entry`. The change is unconditional: `#[unsafe(no_mangle)]` is valid on Rust >= 1.82 in both edition 2021 and edition 2024. No other bare `#[no_mangle]` / `#[export_name]` / `#[link_section]` is emitted; the existing `unsafe extern "C"` shim blocks are already edition-2024-correct and are unchanged.

### Options Considered

| Option | Verdict |
|--------|---------|
| Emit `#[unsafe(no_mangle)]` unconditionally | ✓ Chosen — valid on >= 1.82 in both editions; unconditional is simpler and future-proof; proved correct by the `dispatch.rs` test loading a macro-generated edition-2024 `.so` and running the full cycle |
| Gate the attribute on call-site edition inside the macro | ✗ Rejected — proc-macros cannot reliably know the consumer's edition; fragile and unnecessary |
| Keep `#[no_mangle]`; hold test-udf crates at edition 2021 | ✗ Rejected — defeats the one-edition goal; call-site error would persist for any future edition-2024 consumer |

### Consequences

The macro emits `#[unsafe(no_mangle)]` for every generated UDF entry point. Edition-2024 call-site crates compile without error. The `dispatch.rs` integration test loads a real macro-generated edition-2024 `.so` and drives it through the `__exa_udf_entry` loader, proving the entry point still exports and dispatches correctly.

---

## ADR-037: Resolver symlinks produced by in-build staging-dir tar, superseding host-python3 patch

**Date:** 2026-06-15
**Plan:** `add-dns-name-resolution`
**Status:** Accepted

### Context

Exasol UDFs run in a sandbox that bind-mounts the database's resolver config at `/conf/`. For DNS to work, the SLC root filesystem must present `/etc/hosts` and `/etc/resolv.conf` as symlinks into `/conf/`. Two failure modes block creating these symlinks in the image directly: `COPY` dereferences a dangling symlink into a 0-byte file, and `RUN ln -sf /conf/... /etc/...` hits Docker's build-time bind-mount of those two paths. A prior, never-committed approach worked around this with a host-side `python3` tarball patch invoked from three places (install script, IT harness, CI), introducing an undeclared `python3` dependency and triplicated, drift-prone logic.

### Decision

Produce `/etc/hosts → /conf/hosts` and `/etc/resolv.conf → /conf/resolv.conf` inside the Docker build, in a `packager` stage that copies the runtime root into a staging directory, runs `ln -sf` there (the staging dir is NOT bind-mounted, so `ln` succeeds), and `tar`s it with Alpine's own pinned `tar` (which records symlinks as-is, with no `COPY` dereference). An `artifact` stage (`FROM scratch`) exposes the resulting `lc-rs.tar.gz` for `docker build --output`. `python3` leaves the SLC packaging path entirely. Verified 2026-06-15 to produce byte-for-byte the same symlink entries the python script produced.

### Options Considered

| Option | Verdict |
|--------|---------|
| Staging-dir `tar` inside the Docker build (`packager` stage) | ✓ Chosen — staging dir is not bind-mounted (so `ln` works) and `tar` records symlinks as-is; verified to produce identical entries; one self-contained location for all packaging logic |
| Live symlink at `/etc/hosts`/`/etc/resolv.conf` in the image | ✗ Rejected — `COPY` dereferences the dangling symlink into a 0-byte file; `RUN ln -sf` hits Docker's build-time bind-mount of those two paths |
| Host-side `python3` tarball patch (prior, never-committed approach) | ✗ Rejected — undeclared `python3` dependency plus triplicated, drift-prone logic invoked from install.sh, IT harness, and CI |

### Consequences

The SLC tarball has proper symlink entries (`lrwxrwxrwx ./etc/resolv.conf -> /conf/resolv.conf`, `lrwxrwxrwx ./etc/hosts -> /conf/hosts`) baked in by the Docker build with no host-side post-processing. DNS resolves inside the UDF sandbox. The project memory note `slc-tarball-symlink-patch.md` is superseded — the host-patch approach is no longer used. Spec scenario `container/slim-image / SLC tarball ships the /conf resolver symlinks` asserts the observable tarball behavior.

---

## ADR-038: SLC distribution tarball is the build artifact; all consumers read it, none patch

**Date:** 2026-06-15
**Plan:** `add-dns-name-resolution`
**Status:** Accepted

### Context

Previously each consumer of the SLC (install.sh, IT harness, CI) independently ran `docker save`/`docker export | gzip` to produce a flattened tarball, and the now-superseded approach additionally required a host-side python3 patch step. Every consumer reran the export step, duplicating and drifting the flatten-and-gzip logic.

### Decision

Make `lc-rs.tar.gz` (produced by the `artifact` stage with `docker build --target artifact --output type=local,...`) the single build artifact. The IT harness reads it via `SLC_TARBALL`; `install.sh`, `ci-it-local.sh`, and `ci.yml` produce it with `docker build --target artifact --output` and upload/consume it directly. `docker save`/`docker load`/`docker export` per-consumer steps are dropped.

### Options Considered

| Option | Verdict |
|--------|---------|
| Single `lc-rs.tar.gz` artifact from `artifact` stage; all consumers read it | ✓ Chosen — one source of truth; eliminates drift; no per-consumer export round-trip; symlinks already baked in (ADR-037) |
| Keep `docker save` (CI image artifact) + per-consumer `docker export \| gzip` | ✗ Rejected — every consumer reran the export step, which drifts and duplicates the flatten-and-gzip logic; does not solve the symlink problem |

### Consequences

A single, self-contained `lc-rs.tar.gz` artifact is produced once and read by every consumer without modification. The IT harness reads the path from `SLC_TARBALL` and fails fast with clear guidance if it is unset — a missing tarball is a setup error, not a condition to recover from silently. The `docker save`/`docker load`/`docker export` steps are removed from all consumers.

---

## ADR-036: Spec-delta-free plans are legitimate; specs must not encode implementation detail

**Date:** 2026-06-15
**Plan:** `change-toolchain-edition-2024`
**Status:** Accepted

### Context

The initial drafts of this plan authored spec deltas asserting specific version strings (`rust:1.91-bookworm` → `rust:1.92-bookworm`, `channel = "1.84"` → `"1.92"`), edition literal values, and the macro attribute spelling (`#[no_mangle]` vs `#[unsafe(no_mangle)]`) directly in scenario specs. These are implementation details — not developer-observable behavior. Encoding them in specs creates maintenance debt: every toolchain bump would require a spec delta and a re-record cycle, and the spec library would become a mirror of Cargo.toml and Dockerfile rather than a description of system behavior.

### Decision

Remove all three spec delta files from this plan, making it a spec-delta-free plan (`plan.md` + `decision-log.md` only). Specs describe developer-observable behavior; version numbers, toolchain channels, Docker image tags, attribute spellings, edition values, and Cargo manifest fields are implementation details and must not appear in scenario specs. A plan that carries only a decision log and implementation tasks is legitimate when the work has no observable behavioral delta. As a recording-phase cleanup, version literals that had leaked into prior specs are stripped from the permanent spec library on record.

### Options Considered

| Option | Verdict |
|--------|---------|
| Spec-delta-free plan; strip leaked version literals from existing specs on record | ✓ Chosen — keeps the spec library focused on behavior; eliminates maintenance debt from version-literal churn |
| Keep the spec deltas with version-string assertions | ✗ Rejected — version strings are implementation detail; every future toolchain bump would force a spec record cycle |
| Rewrite the deltas to describe behavior abstractly | ✗ Rejected — at this granularity the change is pure implementation detail (toolchain version, edition value, FFI attribute spelling) with no observable behavioral surface to assert |

### Consequences

The spec library's scenario assertions describe what the system does, not how it is built. Implementation details (toolchain version, Rust edition, Docker image tag, FFI attribute form) live only in code and `plan.md` / `decision-log.md` history. Spec-delta-free plans are an established pattern for infrastructure-level changes that preserve all shipped behavior. Prior version-literal leakage (the "Rust toolchain is pinned" scenario with `channel = "1.84"`, the `[workspace.dependencies]` scenario with enumerated version numbers) is cleaned from the permanent spec library during this recording.

---

## ADR-039: Surface UDF errors via a vtable `run` out-pointer, not a trait method

**Date:** 2026-06-16
**Plan:** `fix-surface-udf-error-messages`
**Status:** Accepted

### Context

When a UDF `run` returns `Err(UdfError)`, the generated run shim mapped the error to exit code `1` and discarded the `UdfError` value entirely. The host (`dispatch.rs`) then found `None` in `take_last_error` (written only by connect-back failures), so the DB saw only `"UDF run returned error code 1"` with no detail. An early plan draft proposed adding a `record_error(&self, &str)` default method to the `UdfContext` trait to carry the error text out, but this was rejected by the user as widening the public trait surface unnecessarily.

### Decision

Add a second parameter `error_out: *mut *mut c_char` to the `ExaUdfVTable.run` function pointer. The generated run shim writes a heap-allocated, host-freed C string holding the error's display text to `*error_out` on the `Err` arm (when `error_out` is non-null) and returns the non-zero error code. The host passes `&mut error_ptr`, reads the text after a non-zero return, and frees the allocation using the `malloc`/`libc::free` C-allocator convention, consistent with all other vtable result strings. `EXA_UDF_ABI_VERSION` is bumped from 3 to 4. The `UdfContext` trait and all bridge `last_error`/`take_last_error`/`record_error` plumbing are untouched.

### Options Considered

| Option | Verdict |
|--------|---------|
| Add `error_out: *mut *mut c_char` out-pointer to the vtable `run` slot; bump ABI to 4 | ✓ Chosen — keeps the error channel inside the ABI the host already owns; zero UDF source changes (recompile only); reuses the established caller-freed-C-string convention; leaves the `UdfContext` trait and connect-back `last_error` sink independent |
| Add `record_error(&self, &str)` default method to the `UdfContext` trait | ✗ Rejected — widens the public trait surface; requires all bridge implementations to be aware of the UDF-error path; rejected by the user |
| Encode the error string into the `i32` return code | ✗ Rejected — the return is a status code and cannot carry text |
| Store the error in a thread-local | ✗ Rejected — an explicit out-pointer owned by the host is clearer and matches the existing single-call hook convention |

### Consequences

The `ExaUdfVTable.run` slot has a two-parameter signature from ABI version 4 onwards. All `.so` files compiled against ABI v3 are rejected at load time with a clear version-mismatch error, not silent UB. UDF authors require only a recompile — no source changes. The connect-back `last_error` channel remains the exclusive error sink for connect-back failures; the UDF-error path is fully independent.

---

## ADR-040: Connect-back is fully supported in SCALAR scripts — the shared run loop and the loopback-address fix (ADR-029) make the "never SCALAR" rule obsolete

**Date:** 2026-06-17
**Plan:** `add-scalar-connect-back`
**Status:** Accepted

### Context

The project CLAUDE.md contained the rule: "Use `SET SCRIPT ... EMITS (...)` for any connect-back UDF; **never** `SCALAR` (SCALAR → SIGABRT mid-execution)." This rule was written as a precaution after the historical SIGABRT traced to `connect_back_sql_address()` returning `localhost:8563`, which routes to Exasol's internal CoreDB proxy and links the new session to the SQL worker (Part:40), causing a SIGABRT within seconds. That root cause was fixed in ADR-029 by switching to `<container-eth0-ip>:8563` via `getifaddrs`. The "never SCALAR" rule was never re-evaluated after ADR-029. A spike on 2026-06-17 proved empirically: (1) Python3 SCALAR connect-back returned `42` with no crash; (2) Rust SCALAR connect-back (`connect-back-scalar` crate, `RUST SCALAR SCRIPT connect_back_scalar() RETURNS BIGINT`) returned `42` with all 15 `db_roundtrip` integration scenarios passing. Code inspection of `crates/exa-udf-runtime/src/dispatch.rs` confirmed that scalar (`ExactlyOnce`) and set (`Multiple`) UDFs share one identical run loop — connect-back (MT_IMPORT exchange, external session) is transport-layer behaviour, not UDF-type-layer behaviour.

### Decision

Connect-back is a first-class supported capability for both `SCALAR` and `SET/EMITS` Rust scripts. The "never SCALAR" restriction is removed from CLAUDE.md and replaced with a positive statement: both script types support connect-back; the address rule (`cluster_ip()`, never loopback) and the transaction-conflict rule remain unchanged and apply to both. No runtime code change was required — the dispatch path was already correct. ADR-040 records the proof chain so this question cannot be re-litigated from git history alone.

### Options Considered

| Option | Verdict |
|--------|---------|
| No runtime code change — SCALAR connect-back works as-is; relax the CLAUDE.md rule | ✓ Chosen — empirically verified; zero-change to runtime; removes a stale prohibition that would cause authors to write less capable UDFs unnecessarily |
| Add a scalar-specific connect-back verification step or fast-path | ✗ Rejected — unnecessary complexity; the run loop is already shared; the spike produced no crash |
| Keep the "never SCALAR" rule with a clarifying footnote | ✗ Rejected — a rule that contradicts itself with a footnote is noise; the rule must be authoritative |
| No ADR — treat as a minor docs cleanup | ✗ Rejected — without a permanent record, the restriction will inevitably be re-introduced by a future planner seeing the old commit history without the spike output |

### Consequences

SCALAR connect-back UDFs are a supported pattern. Authors may choose `SCALAR SCRIPT ... RETURNS ...` or `SET SCRIPT ... EMITS (...)` based solely on UDF logic, not on a connect-back restriction. The address rule (`cluster_ip()`, never `127.0.0.1`) and the `std::process::exit(0)` lifecycle rule apply equally to both script types. The `connect-back-scalar` crate serves as the canonical example and integration test fixture for the scalar connect-back path.

---

## ADR-041: Thread an emit-flusher closure into HostContextBridge, mirroring conn_requester

**Date:** 2026-06-19
**Plan:** `fix-emit-buffer-and-result-streaming`
**Status:** Accepted

### Context

The prior `ctx.emit` accumulated rows in `EmitBuffer` indefinitely — there was no size check, so a UDF emitting millions of rows per input batch would grow the buffer without bound. A flush was only sent after `run()` returned, so peak memory was the full batch output. A mechanism was needed to flush mid-run when the buffer crossed a threshold, matching the C++ SLC reference (`SWIG_MAX_VAR_DATASIZE = 4_000_000`).

### Decision

Add a `flusher: EmitFlusher` closure to `HostContextBridge`, threaded in by `run_batch` exactly like the existing `conn_requester`. `HostContextBridge::emit` pushes the row then invokes the flusher when `should_flush()` is true.

### Options Considered

| Option | Verdict |
|--------|---------|
| Thread a `flusher` closure into the bridge (mirroring `conn_requester`) | ✓ Chosen — reuses the established closure-injection pattern; keeps the ZMQ socket out of `EmitBuffer`; `EmitBuffer` stays a pure, trivially-unit-testable data type |
| Give `EmitBuffer` an owned `Option<Box<dyn FnMut>>` flush callback | ✗ Rejected — pulls socket-touching behavior into what should stay a pure data type |

### Consequences

The bridge now carries two closures: `conn_requester` (connect-back) and `flusher` (emit). Both closures are injected by `run_batch` and must share the same `RefCell<&mut Protocol>` (see ADR-042). `EmitBuffer` remains a pure data structure with no I/O dependencies.

---

## ADR-042: Both bridge closures share one RefCell<&mut Protocol>

**Date:** 2026-06-19
**Plan:** `fix-emit-buffer-and-result-streaming`
**Status:** Accepted

### Context

The emit flusher (ADR-041) and the existing `conn_requester` both need `&mut Protocol` to send wire messages. In `run_batch`, both closures are live at the same time. A sound mechanism was needed to share access to the single `&mut Protocol`.

### Decision

The emit flusher and the connect-back `conn_requester` borrow the same `RefCell<&mut Protocol>` in `run_batch`. Calls are strictly serial because the dispatch loop is blocked inside `run_batch` awaiting the UDF function return.

### Options Considered

| Option | Verdict |
|--------|---------|
| Single shared `RefCell<&mut Protocol>` for both closures | ✓ Chosen — calls are serial; one cell yields non-overlapping borrows; simplest sound option |
| Two separate `RefCell`s over the same `&mut Protocol` | ✗ Rejected — aliasing a unique borrow is unsound |
| `Rc<RefCell<>>` | ✗ Rejected — unnecessary heap/refcount overhead for a strictly-serial call pattern |

### Consequences

A single `RefCell<&mut Protocol>` is shared between `flusher` and `conn_requester`. Simultaneous borrows cannot occur because the dispatch loop is blocked during UDF execution, so the serial constraint is structural rather than enforced at compile time.

---

## ADR-043: query_for_each as a default trait method; query delegates to it

**Date:** 2026-06-19
**Plan:** `fix-emit-buffer-and-result-streaming`
**Status:** Accepted

### Context

`ExaConnection::query` (and the runtime override) called `Connection::query` which internally called `fetch_all`, materializing the entire result set into a `Vec<RecordBatch>` before returning. A streaming API was needed so UDFs could process result sets without holding the entire result in memory. The design had to keep every existing `ExaConnection` impl (including test mocks) compiling unchanged.

### Decision

Add `query_for_each<F: FnMut(Vec<Value>) -> Result<(), UdfError>>` to `ExaConnection` with a default implementation over `query_arrow` that converts each batch's rows and invokes the callback. Re-express the default `query` to collect via `query_for_each`, sharing one code path. The runtime overrides `query_for_each` with the true streaming path; `query` automatically delegates to that override.

### Options Considered

| Option | Verdict |
|--------|---------|
| `query_for_each` as a default trait method; `query` delegates to it | ✓ Chosen — backward compatible; every existing impl works unchanged; `query` and streaming cannot diverge; one conversion code path |
| A separate streaming trait | ✗ Rejected — breaks existing mock impls; requires double-implementation for the runtime |
| A free function | ✗ Rejected — does not allow the runtime override to automatically fix `query`'s materialization behavior |

### Consequences

Every `ExaConnection` impl that provides only `query_arrow` automatically gets streaming behavior via the default. `query` is no longer materialization-primary — it delegates to `query_for_each` and collects. A mock that implements only `query_arrow` passes both `query` and `query_for_each` tests without change.

---

## ADR-044: Stream connect-back via execute + ResultSetIterator with fetch_all fallback due to current_thread constraint

**Date:** 2026-06-19
**Plan:** `fix-emit-buffer-and-result-streaming`
**Status:** Accepted

### Context

`RuntimeExaConnection::query_for_each` needed to stream one `RecordBatch` at a time rather than materializing the full result. exarrow-rs 0.12.7 exposes `Connection::execute -> ResultSet`, `ResultSet::into_iterator -> ResultSetIterator`, and `ResultSetIterator::next_batch`. The plan specified driving `next_batch` in a loop. During implementation, a `current_thread` Tokio runtime constraint was discovered: `next_batch` calls `Handle::try_current()` then `handle.block_on(fetch_next_batch())`. Calling `handle.block_on()` from within an outer `Runtime::block_on(async{})` deadlocks — the single thread is already occupied.

### Decision

`RuntimeExaConnection::query_for_each` uses `Connection::execute(sql)` followed by `fetch_all()` inside one `block_on`, then iterates the owned `Vec<RecordBatch>` with `into_iter()`, dropping each batch before processing the next. This eliminates the double-copy peak (Arrow + Value simultaneously) of the old `query` implementation, though it does not achieve true per-batch server streaming. Per-batch streaming requires a future exarrow-rs API that can be polled without a nested `handle.block_on` call.

### Options Considered

| Option | Verdict |
|--------|---------|
| `execute` + `fetch_all` inside one `block_on`, iterate owned batches | ✓ Chosen — eliminates the double-copy peak; no deadlock; works with the current `current_thread` runtime; no exarrow-rs version bump |
| Drive `ResultSetIterator::next_batch` per-batch on the `current_thread` runtime | ✗ Rejected — deadlocks: `next_batch` calls `handle.block_on()` from within an outer `block_on` on a single-thread runtime |
| Add a streaming API to exarrow-rs upstream | ✗ Rejected — no version bump needed; 0.12.7 already exposes the closest available primitive |

### Consequences

True per-batch server streaming (dropping batch _N_ before fetching batch _N+1_ from the server) is not achieved — all batches are fetched before row-by-row processing begins. However, the per-batch drop loop eliminates the simultaneous Arrow + Value peak of the prior `query` implementation. Per-batch server streaming requires a future exarrow-rs API change. This deviation from the plan is documented in the verification report.

---

## ADR-045: Per-UDF `__exa_udf_entry_<NAME>` symbols, no registry

**Date:** 2026-06-19
**Plan:** `add-multi-udf-entry-points`
**Status:** Accepted

### Context

Each annotated function in a UDF crate needed its own unique ABI entry point so that one `.so` could host multiple UDFs. Two approaches were considered: emitting a single registry symbol that returns a name-to-vtable table, or emitting one `#[no_mangle]` symbol per UDF suffixed with the SQL name derived from the function identifier.

### Decision

Each annotated function exports its own `#[unsafe(no_mangle)]` `__exa_udf_entry_<NAME>` symbol. The loader resolves exactly one by the DB-supplied `script_name`. No registry symbol or name-to-vtable table is emitted.

### Options Considered

| Option | Verdict |
|--------|---------|
| Per-UDF `__exa_udf_entry_<NAME>` symbols | ✓ Chosen — a direct `dlsym` by script name needs no table format, no allocation, and no new ABI to version; the linker rejects same-name duplicates for free |
| Single registry symbol returning a name→vtable table | ✗ Rejected — requires a table format, allocation, and a new registry ABI to version; does not leverage linker duplicate detection |

### Consequences

One `.so` may export many UDFs, each addressable by the SQL script name the database sends in the handshake. The loader shape is unchanged — it still performs a single `dlsym` per session. A same-name duplicate in one crate is a link-time error, not a silent wrong-UDF selection.

---

## ADR-046: Hard-break the bare `__exa_udf_entry` symbol — no fallback

**Date:** 2026-06-19
**Plan:** `add-multi-udf-entry-points`
**Status:** Accepted

### Context

The macro previously emitted a bare `__exa_udf_entry` symbol (no suffix). Removing it breaks all `.so` artifacts compiled against SDK < 0.14.0. Two options were considered: maintain backward compatibility via a fallback to the bare symbol, or hard-break with an explicit rebuild-hint error.

### Decision

The macro stops emitting `__exa_udf_entry`. The loader never falls back to it. Legacy `.so` files fail at load time with `no entry point found for script '<NAME>'; hint: rebuild with sdk >= 0.14.0`.

### Options Considered

| Option | Verdict |
|--------|---------|
| Hard-break: remove bare symbol; clear rebuild-hint error | ✓ Chosen — interview decision; a silent fallback is dangerous once a `.so` carries multiple UDFs (which UDF would the bare symbol mean?); an actionable error is safer than ambiguous behavior; the project is pre-1.0 so a clean break is acceptable |
| Keep bare symbol; fall back when named symbol absent | ✗ Rejected — would silently load the wrong UDF in a multi-UDF `.so`; masks author error |

### Consequences

All `.so` artifacts built against SDK < 0.14.0 must be rebuilt. The rebuild-hint error message is surfaced through the protocol close path with the `F-UDF-CL-RUST-` prefix. The MINOR version bump (ADR-047) signals the breaking change.

---

## ADR-047: SQL name derived from function identifier via ASCII UPPER_SNAKE_CASE

**Date:** 2026-06-19
**Plan:** `add-multi-udf-entry-points`
**Status:** Accepted

### Context

Each `#[exasol_udf]`-annotated function needed an SQL entry-point name to suffix its generated symbols and match the DB's `script_name`. Three derivation options were considered: always require an explicit `name = "..."` attribute, keep the identifier verbatim (case-sensitive), or derive from the function identifier by uppercasing.

### Decision

The default SQL name is `fn_ident.to_uppercase()` (underscores preserved), matching Exasol's default identifier uppercasing. A `name = "..."` attribute overrides the derived name verbatim.

### Options Considered

| Option | Verdict |
|--------|---------|
| Derive via ASCII `UPPER_SNAKE_CASE` from fn identifier; `name=` overrides | ✓ Chosen — `fn double_it` → `DOUBLE_IT` naturally matches `CREATE SCRIPT DOUBLE_IT`; zero-config for the common case; `name=` covers quoted or unusual identifiers |
| Always require explicit `name = "..."` | ✗ Rejected — unnecessary boilerplate for the common case where the function name matches the SQL script name |
| Keep identifier verbatim (case-sensitive) | ✗ Rejected — Exasol object names are upper-cased by default; `fn double_it` would not match `CREATE SCRIPT DOUBLE_IT` without quoting |

### Consequences

Authors annotating `fn double_it` get `__exa_udf_entry_DOUBLE_IT` for free. The `name = "..."` attribute is the escape hatch for quoted identifiers or any name that does not follow `UPPER_SNAKE_CASE`. The derived name must equal the bare object name the database sends as `script_name`; `script_schema` is not part of the symbol.

---

## ADR-048: Declared EMITS ColumnMeta (not Arrow schema) is authoritative for the target proto block

**Date:** 2026-06-24
**Plan:** `add-emit-batch-arrow`
**Status:** Accepted

### Context

`EmitBuffer::push_batch` must decide which proto type block (double_data, int32_data, string_data, …) to pack each Arrow column's values into. Two approaches were considered: derive the target block from the Arrow `DataType` alone, or use the declared EMITS output `ColumnMeta` (the same `ExaType` the row-based `to_proto` path already uses).

### Decision

`push_batch` dispatches on the Arrow `DataType` only to extract each cell value, then packs it into the proto block dictated by the declared output `ExaType` — identical to the existing `to_proto` row path. The declared EMITS schema is the single source of truth for the target proto block.

### Options Considered

| Option | Verdict |
|--------|---------|
| Declared EMITS `ColumnMeta` (`ExaType`) dictates the target proto block | ✓ Chosen — Arrow `DataType` is ambiguous for Exasol: `Utf8` maps to VARCHAR/CHAR/GEOMETRY/HASHTYPE/INTERVAL; only the declared schema names the correct block; also keeps `push_batch` byte-identical to the row path |
| Derive target block purely from Arrow `DataType` (no `ColumnMeta`) | ✗ Rejected — ambiguous: multiple Exasol types collapse onto one Arrow type; produces wrong block for extended Exasol types without consulting the declared schema |

### Consequences

`push_batch` requires the `&[ColumnMeta]` slice at call time (the same slice the flusher serialises with), making the output schema an explicit dependency of the Arrow batch-emit path. The `HostContextBridge` carries `output_meta` in its struct fields, threaded in by dispatch at construction.

---

## ADR-049: Standalone emit-arrow feature; connect-back implies it

**Date:** 2026-06-24
**Plan:** `add-emit-batch-arrow`
**Status:** Accepted

### Context

Arrow batch emit (`emit_batch`) requires the `arrow` crate. The question was whether to gate it under the existing `connect-back` feature or add a new independent feature so pure-compute UDFs (no connect-back) can emit Arrow batches without pulling in tokio, exarrow-rs, and rustls.

### Decision

Add `emit-arrow = ["dep:arrow"]` to `exasol-udf-sdk`; add `emit-arrow` to the `connect-back` feature list so connect-back continues to pull in arrow transitively. Building with neither feature compiles no `arrow` dependency.

### Options Considered

| Option | Verdict |
|--------|---------|
| New standalone `emit-arrow` feature; `connect-back` implies it | ✓ Chosen — minimal dependency surface for pure-compute UDFs; connect-back UDFs get it for free; mirrors the SDK's existing pattern of additive, independent feature flags |
| Gate `emit_batch` directly under `connect-back` | ✗ Rejected — forces pure-compute UDFs to pull in tokio/exarrow-rs/rustls to emit Arrow batches; violates the principle of minimal dependency surface |

### Consequences

UDF crates that want only Arrow batch emit add `emit-arrow` to their `exasol-udf-sdk` dependency and get no transitive tokio or exarrow-rs. Connect-back UDFs gain `emit_batch` automatically. The `arrow` dependency is optional (no implicit compile cost for `emit`-only UDFs).

---

## ADR-050: Vectorised column-at-a-time Arrow encoding with row-granular 4 MB split points

**Date:** 2026-06-24
**Plan:** `add-emit-batch-arrow`
**Status:** Accepted

### Context

`EmitBuffer::push_batch` must encode an Arrow `RecordBatch` into proto type blocks without a `Vec<Value>` intermediate, while respecting the hard 4 MB `MT_EMIT` wire limit. A batch's serialised size is unknown at compile time and can exceed 4 MB, so flushing strictly at batch boundaries is incorrect. The proto type blocks use a dense, row-major-interleaved layout (non-null cells only) that must be preserved for `from_proto` to decode correctly.

### Decision

Encode column-at-a-time (vectorised): downcast each Arrow column array once and read its null buffer once in bulk, then assemble values row-by-row into the interleaved proto blocks. A cheap cumulative per-row byte cost (fixed-width by width; variable-width from the Arrow offset buffer; Decimal/Timestamp via the same fixed estimate as `value_byte_cost`) is computed without touching data bytes. Row-granular split points at the 4 MB threshold yield zero-copy `RecordBatch::slice` segments that are encoded and flushed; the sub-4 MB trailing remainder is materialised once into the shared `Vec<Value>` buffer so the row path and the end-of-run tail flush remain coherent.

### Options Considered

| Option | Verdict |
|--------|---------|
| Vectorised column-at-a-time downcast + row-major block assembly + row-granular 4 MB splits | ✓ Chosen — eliminates per-cell downcast and Value allocation for the bulk of the data; offset-buffer prefix-sum gives split points without touching data bytes; correct for all EMITS schemas including multi-column same-ExaType and nulls |
| Flush strictly at batch boundaries | ✗ Rejected — a batch can exceed the hard 4 MB wire cap; incorrect, not merely slow |
| Cell-by-cell extraction (downcast per cell, per-cell Value allocation) | ✗ Rejected — the per-row pivot the feature exists to remove; equivalent to the existing `record_batch_to_rows` path |
| True bulk `extend` of whole columns into blocks | ✗ Rejected — scrambles the row-major-interleaved block layout whenever multiple columns share a block type or nulls are present; not correct for arbitrary EMITS schemas |

### Consequences

`push_batch` is faster than the row pivot for large batches (one downcast per column, not per cell; no intermediate `Value` vec for bulk data). The trailing <4 MB remainder is the only `Vec<Value>` materialisation on the batch path — bounded by the same 4 MB the row path already buffers. The row `emit`/`to_proto` path and all flush semantics are unchanged, keeping regression surface minimal.

---

## ADR-051: Remove `query_arrow`; no Arrow accessor replacement (#26)

**Date:** 2026-06-25
**Plan:** `fix-abi-feature-safety`
**Status:** Accepted

### Context

`ExaConnection::query_arrow` returned `Vec<arrow::RecordBatch>` across the `.so` boundary. A UDF `.so` and the host each link their own static `arrow`, so `TypeId`/vtable comparisons on those batches silently return wrong values — no error, no panic, just corrupted data (issue #26). The Arrow IPC emit-throughput benchmark also showed Arrow IPC ser/deser is only 2–9% of `emit_batch`'s cost, so an Arrow streaming path buys nothing over `Vec<Value>`.

### Decision

Drop `ExaConnection::query_arrow`. Make `query_for_each` (`Vec<Value>` row callback) the required streaming method; `query` defaults to collecting it. Do not add a `query_arrow_ffi` or Arrow C Data Interface replacement — `Vec<Value>` is already safe and ergonomic.

### Options Considered

| Option | Verdict |
|--------|---------|
| Remove `query_arrow`, no replacement | ✓ Chosen — eliminates the footgun; `Vec<Value>` is sufficient and safe |
| `#[deprecated]` but keep | ✗ Rejected — deprecated unsafe API still compiles; silent UB still reachable |
| Replace with `query_arrow_ffi` (Arrow C Data Interface) | ✗ Rejected — Arrow C Data Interface scope dropped; benchmark showed no throughput gain |
| Gate behind a feature | ✗ Rejected — vestigial gated unsafe API re-introduces the hazard for feature-enabled builds |

### Consequences

`ExaConnection` becomes arrow-free. All connect-back results are delivered as `Vec<Value>` rows — the same type that crosses every other SDK boundary. This is also the prerequisite for making `UdfContext` feature-independent (see ADR-052): without `query_arrow`, the `connect_back` module needs no optional `arrow` dependency, so it can compile unconditionally.

---

## ADR-052: Make the `UdfContext` trait-object vtable feature-independent (#31)

**Date:** 2026-06-25
**Plan:** `fix-abi-feature-safety`
**Status:** Accepted

### Context

The `.so`↔host call context crosses as a `&mut dyn UdfContext` Rust trait object inside the run shim. That trait-object vtable is ordered by method declaration. Several `UdfContext` methods were `#[cfg(feature = ...)]`-gated: `cluster_ip`/`connection`/`connect_back` behind `connect-back`, and `emit_record_batch_ipc` behind `emit-arrow`. A UDF `.so` built without `connect-back` but with `emit-arrow` places `emit_record_batch_ipc` in an earlier vtable slot than the host (which was built with both features). `ctx.emit_batch()` therefore silently dispatched to `cluster_ip`, returned `Ok`, and emitted 0 rows with no error (issue #31). The ABI fingerprint did not encode feature flags, so the loader did not catch it.

### Decision

Remove every `#[cfg(feature = ...)]` from `UdfContext` method declarations. Declare `cluster_ip`, `connection`, `connect_back`, and `emit_record_batch_ipc` unconditionally with `Unimplemented` defaults so the `dyn UdfContext` vtable layout is identical in all feature configurations. Narrow the `emit-arrow` feature to gate only `dep:arrow` and the `EmitBatch` extension trait; it no longer gates any trait method declaration. Bump `EXA_UDF_ABI_VERSION` 4 → 5 so a `.so` compiled against the old layout fails the loader's version check with a clear `AbiMismatch` error.

### Options Considered

| Option | Verdict |
|--------|---------|
| Unconditional method declarations with `Unimplemented` defaults | ✓ Chosen — eliminates the vtable-skew class entirely; all builds get the same layout |
| Encode feature set in the ABI fingerprint (detect-only) | ✗ Rejected — detect-only still cannot interoperate; structural fix is strictly better |
| Separate `#[repr(C)]` context vtable | ✗ Rejected — heavy; shifts complexity without removing the root cause |

### Consequences

Every `UdfContext` method resolves to the same vtable slot regardless of which cargo features a UDF enables. The `emit-arrow`-only UDF emit-batch dispatch bug (#31) is eliminated at the structural level. Fingerprint-feature encoding remains a possible defense-in-depth follow-up but is not required once the layout is stable. Old `.so` artifacts built against ABI v4 are rejected loudly rather than misdispatching.

---

## ADR-053: Configure UDF verbosity via a `%udf_debug_level` directive, not an env var

**Date:** 2026-06-26
**Plan:** `add-debug-output-redirect`
**Status:** Accepted

### Context

A UDF author chasing a bug needs to raise the SLC's tracing verbosity to `debug` without touching the cluster's process environment. The `tracing-subscriber` subscriber is installed in `main()` before the handshake; the `source_code` field carrying per-script configuration arrives only after the handshake completes. The author controls `CREATE SCRIPT` text but not the SLC process environment inside a production cluster.

### Decision

Declare verbosity as a `%udf_debug_level debug|info|warn|error` directive in the script source, parsed from the `source_code` field of the handshake metadata — the same channel already used by `%udf_object`. The parser defaults to `info` for absent or unrecognised values.

### Options Considered

| Option | Verdict |
|--------|---------|
| `%udf_debug_level` directive in `CREATE SCRIPT` source | ✓ Chosen — the only knob reachable without touching the container's process env |
| Rely solely on `RUST_LOG` env var read at `main()` init | ✗ Rejected — read before the handshake; cannot carry a per-script level |
| `std::env::set_var("RUST_LOG", ...)` before `init()` | ✗ Rejected — env var must be set before `init()`; setting it after has no effect |

### Consequences

Any author who can write `CREATE SCRIPT` SQL can tune SLC verbosity without a container rebuild or environment variable change. The directive is parsed after the handshake, so early `main()`/handshake lines always use the process-default level (`info`).

---

## ADR-054: Apply post-handshake log level via `tracing_subscriber::reload` + `rebuild_interest_cache`

**Date:** 2026-06-26
**Plan:** `add-debug-output-redirect`
**Status:** Accepted

### Context

The `%udf_debug_level` directive is parsed only after the handshake, but the `tracing-subscriber` is already installed in `main()`. The plan originally specified `tracing::level_filters::LevelFilter::set_max_level` as the one-line mechanism to raise the global max level at runtime. During implementation it was found that `set_max_level` does not exist in `tracing 0.1` — the API is not part of the public surface of that version.

### Decision

Install a `tracing_subscriber::reload`-wrapped `EnvFilter` in `main()`. After parsing `%udf_debug_level` post-handshake, call `reload_handle.reload(new_filter)` followed by `tracing::callsite::rebuild_interest_cache()`, which propagates the new level to the callsite interest cache and updates the value returned by `LevelFilter::current()`. This is a one-time mutation (no further reloads); no new crate dependency is added (`tracing-subscriber` already uses `reload` internally and the feature is available). The `reload::Handle` is stored as a field on `Runtime`.

### Options Considered

| Option | Verdict |
|--------|---------|
| `tracing_subscriber::reload` handle + `rebuild_interest_cache()` | ✓ Chosen — works correctly in `tracing 0.1`; one mutation, no extra dependency |
| `tracing::level_filters::LevelFilter::set_max_level` | ✗ Rejected (does not exist) — this API is absent from `tracing 0.1`'s public surface |
| Reinstall the entire subscriber post-handshake | ✗ Rejected — `init()` panics if called twice; requires unsafe global state reset |

### Consequences

The user-facing behavior (one-time post-handshake global level change, no subscriber reinstall) is identical to what the plan specified. The mechanism is `reload` + `rebuild_interest_cache()` rather than the originally cited `set_max_level`. The `reload` feature of `tracing-subscriber` is used but no new crate dependency is introduced. Events before the handshake use the process-default level.

---

## ADR-055: Output redirect is the database's job (fd-level dup2), not an SLC-managed TCP sink

**Date:** 2026-06-26
**Plan:** `add-debug-output-redirect`
**Status:** Accepted

### Context

Previous planning iterations proposed an SLC-managed live-output mechanism: a `%udf_debug_output host:port` directive, a `tracing_subscriber::reload` TCP layer opened post-handshake, plus alloc-error and signal handlers writing a crash report to BucketFS on native abort. The architect corrected this: output redirect is a database function already implemented in `Engine/src/exscript/pluggable/zmqinternal.cc`. The engine reads `SET SESSION SCRIPT OUTPUT ADDRESS`, opens a TCP socket, and `posix_spawn_file_actions_adddup2`s it onto the child's fd 1 (stdout) and fd 2 (stderr) *before* spawning `nschroot` → `exaudfclient`.

### Decision

Rely entirely on the database's `SET SESSION SCRIPT OUTPUT ADDRESS 'host:port'` mechanism. The SLC writes diagnostics to stderr and does nothing else to provide the live stream. No `%udf_debug_output` directive, no SLC TCP connection, no crash-report subsystem. The `runtime/crash-report` spec is deleted. This decision is recorded explicitly so future planners do not re-propose SLC-managed redirect.

### Options Considered

| Option | Verdict |
|--------|---------|
| Use the DB's existing fd-level `SET SESSION SCRIPT OUTPUT ADDRESS` redirect | ✓ Chosen — captures everything an SLC layer would, plus startup failures and hard crashes before any Rust code runs |
| SLC-managed `%udf_debug_output` + reload TCP layer post-handshake | ✗ Rejected — reimplements a working DB feature; cannot capture crashes that occur before the post-handshake layer is installed |
| Two-mechanism crash reporting (alloc/signal handlers, BucketFS PUT) | ✗ Rejected — the fd-2 redirect already delivers panics, aborts, and signal-time stderr to the listener; bespoke subsystem is redundant complexity |

### Consequences

The SLC is simpler: no TCP connection management, no alloc-error hook, no signal handler, no BucketFS write path. The `runtime/crash-report` spec is permanently removed. The DB redirect captures startup errors and hard native aborts that an SLC-side TCP layer installed post-handshake could never see.

---

## ADR-056: UDF logging via `udf_log!` + `UdfContext::debug_level`, writing to stderr

**Date:** 2026-06-26
**Plan:** `add-debug-output-redirect`
**Status:** Accepted

### Context

UDF code runs inside a `dlopen`-loaded `.so`. The `.so` statically links its own copy of `tracing`; its global dispatcher is a separate static from the host's. Cross-`.so` dispatcher sharing relies on the fragile static-identity pattern this project bans (see `dispatch-loader` spec and ADR for the ABI fingerprint). Yet UDF authors need a way to emit level-filtered diagnostic lines that reach the same stderr stream the DB redirect captures.

### Decision

Add a default `fn debug_level(&self) -> tracing::Level { tracing::Level::INFO }` to `UdfContext`. The host `HostContextBridge` implementation returns the session's currently resolved level. Add a `udf_log!(ctx, level, ...)` macro that formats and writes to stderr only when `ctx.debug_level()` permits the level. Writing to fd 2 directly bypasses the cross-`.so` dispatcher problem and lands in exactly the stream the DB redirect captures.

### Options Considered

| Option | Verdict |
|--------|---------|
| `udf_log!` macro writing to stderr, gated by `ctx.debug_level()` | ✓ Chosen — trivially correct; lands in the DB-redirected stream; no cross-`.so` globals |
| Share the runtime's `tracing` dispatcher across the `dlopen` boundary | ✗ Rejected — cross-`.so` static-identity pattern; banned by this project; silently broken when host and `.so` are not compiled with the same `tracing` instance |

### Consequences

UDF authors can emit level-filtered diagnostic lines to stderr without a `.so`-local subscriber or global state. Existing UDFs compile unchanged because `debug_level()` has a default body. The `dyn UdfContext` vtable changes, so the SDK version bumps per project rules and the ABI fingerprint is regenerated; a stale `.so` is rejected with a clear `AbiMismatch` error.
