# Decision Log: add-v1-rust-udf-slim

Date: 2026-06-05

## Interview

**Q:** What does v1 need to prove end-to-end?
**A:** Scalar UDF, Set/EMITS UDF, and a 3rd-party dependency statically linked into the musl artifact (e.g. `serde_json`). Connect-back is NOT in scope for v1.

**Q:** How should the integration tests start Exasol?
**A:** `testcontainers-rs` — spin up `exasol/docker-db:2026.1.0` automatically. Self-contained and CI-friendly.

**Q:** Does the plan cover building the slim Docker image?
**A:** Yes — the full path: `Dockerfile` → local `docker build` → image loaded/used by the testcontainers IT harness.

**Q:** Which Docker image is pinned for the Exasol DB in the integration tests?
**A:** `exasol/docker-db:2026.1.0` (the latest confirmed image in the 2026.x stream).

**Q (carried from the prior bootstrap interview):** How should the bootstrap acquire `zmqcontainer.proto`?
**A:** Fetch via the GitHub raw URL and vendor into `crates/exa-proto/proto/`; no submodule. Record provenance in `PROTO_SOURCES.md`.

**Q (carried):** Which Rust toolchain pin?
**A:** `stable-1.84` exactly — must match the container toolchain for ABI fingerprint determinism.

## Design Decisions

### [1] Fold workspace bootstrap into this plan; supersede add-workspace-bootstrap

- **Decision:** Make workspace + `exa-proto` bootstrap Phase 1 of this single v1 plan; mark `add-workspace-bootstrap` as superseded (removed after Phase 1 is recorded).
- **Alternatives:** Implement `add-workspace-bootstrap` first as its own plan, then layer v1 on top.
- **Rationale:** The user asked to bootstrap "along the way." A single atomic v1 plan avoids a half-applied standalone bootstrap plan drifting out of sync with v1's manifest (which also lists the `test-udfs/*` members and dev-deps the bootstrap plan never knew about).
- **Promotes to ADR:** no

### [2] v1 is Option A (precompiled .so) only; JIT returns an unsupported error

- **Decision:** Implement only the precompiled-`.so` execution path. The runtime's compiler entry point returns an unsupported-feature error for the JIT (Option C) path.
- **Alternatives:** Implement JIT too (the design's "primary" path).
- **Rationale:** JIT requires the ~1.4 GB image, a vendored Cargo registry, and an in-container compile/cache pipeline. Option A exercises the entire protocol, SDK, macro, loader, and dispatch surface with far less infrastructure, which is all v1 needs to prove correctness end-to-end.
- **Promotes to ADR:** yes

### [3] Exclude connect-back entirely from v1

- **Decision:** Do not implement `ExaConnection`, `exa()`/`exa_named()`/`exa_connect()`, the tokio runtime, or credential handling. The `connect-back` Cargo feature is declared but compiles to nothing.
- **Alternatives:** Wire the connect-back trait and feature now.
- **Rationale:** Connect-back pulls in `tokio` + `exarrow-rs` on the UDF/runtime side and a whole credential-resolution path (`PB_IMPORT_CONNECTION_INFORMATION`, named `CONNECTION` objects) that none of the three v1 proof scenarios need. Keeping it out shrinks the v1 surface and avoids async/sync-duality risk.
- **Promotes to ADR:** yes

### [4] testcontainers-rs with a pinned DB image, privileged mode, real-DB assertions

- **Decision:** ITs use `testcontainers-rs` to start `exasol/docker-db:2026.1.0` with `with_privileged(true)`, exposing DB port `8563` and BucketFS port `2580`, and assert via `exarrow-rs` SQL. Tests are gated behind an `integration` feature.
- **Alternatives:** Manual docker-compose harness; the script-languages emulator (no real DB/BucketFS).
- **Rationale:** Self-contained, CI-friendly, RAII teardown. The emulator cannot prove the BucketFS upload + `ALTER SESSION` + `CREATE SCRIPT` + `SELECT` path, which is exactly what v1 must demonstrate. The DB image needs `--privileged` (confirmed by Exasol docker-db docs).
- **Promotes to ADR:** yes

### [5] Pin the DB image to 2026.1.0 rather than 2026.latest

- **Decision:** Pin `exasol/docker-db:2026.1.0` in the harness.
- **Alternatives:** Use `2026.latest` as CLAUDE.md suggests.
- **Rationale:** Reproducible, deterministic integration tests; `2026.1.0` is the confirmed latest tag in the 2026.x stream. A floating `latest` tag would make IT outcomes non-reproducible across runs.
- **Promotes to ADR:** no

### [6] BucketFS upload via direct HTTP PUT; SQL via exarrow-rs directly

- **Decision:** The harness uploads `.so` artifacts with an HTTP PUT to `http://w:<write-password>@<host>:<bucketfs-port>/<bucket>/<path>` (reqwest) and runs all SQL through `exarrow-rs` with `validate_server_certificate(false)`.
- **Alternatives:** Shell out to `exapump` for upload and SQL.
- **Rationale:** `exapump` is a CLI, not a library; shelling out adds a process dependency and brittle parsing. BucketFS's HTTP PUT API and `exarrow-rs`'s async connection API are both directly callable from the test crate, which is cleaner and matches the project's preference for `exarrow-rs` as the Rust driver. Project rule "do not validate SSL certificates" maps to `validate_server_certificate(false)`.
- **Promotes to ADR:** yes

### [7] Build test UDFs with raw cargo --target musl, not cargo-exaudf

- **Decision:** Build the three test UDFs via `cargo build --release --target x86_64-unknown-linux-musl -p <crate>`; `cargo-exaudf` stays a stub.
- **Alternatives:** Implement enough of `cargo-exaudf build` to produce the musl `.so`.
- **Rationale:** `cargo-exaudf` (scaffold/build/validate) is explicitly out of v1 scope. Raw `cargo build --target` produces the identical artifact the slim image loads; the musl target is added via `rust-toolchain.toml` `targets`.
- **Promotes to ADR:** no

### [8] Pure I/O-free protocol state machine separated from ZMQ transport

- **Decision:** `exa-zmq-protocol::Protocol` consumes decoded `ExascriptResponse` and produces `ExascriptRequest`/`HostEvent` with no socket I/O; the DEALER socket lives only in `ZmqTransport`.
- **Alternatives:** Fold socket I/O directly into the state machine.
- **Rationale:** Makes the entire message-ordering logic unit-testable with fixtures (the subtlest part of the system), and isolates the one piece that must talk to libzmq. This is the design doc's stated key decision.
- **Promotes to ADR:** yes

### [9] Single C-ABI crossing with ABI-version + fingerprint gating

- **Decision:** The only FFI boundary is `extern "C" fn __exa_udf_entry() -> *const ExaUdfVTable`. The loader checks `abi_version == 1` and the `sdk_fingerprint` before calling `create`; the macro embeds a build.rs-baked fingerprint and wraps `run` in `catch_unwind`.
- **Alternatives:** Pass rich trait objects across the boundary; skip the fingerprint check.
- **Rationale:** Rust has no stable ABI; passing trait objects across a dlopen boundary is UB-prone. A `#[repr(C)]` vtable plus a fingerprint check turns a toolchain mismatch into a clear error instead of UB, and `catch_unwind` prevents unwinding across FFI.
- **Promotes to ADR:** yes

### [10] Pin toolchain to stable-1.84 and patch exarrow-rs via [patch.crates-io]

- **Decision:** `rust-toolchain.toml` pins `channel = "1.84"` (with the musl target); the root `Cargo.toml` patches `exarrow-rs` to its local path even though connect-back is out of v1.
- **Alternatives:** Float toolchain; add exarrow-rs patch later.
- **Rationale:** The slim image builds `FROM rust:1.84-bookworm`; the `EXA_SDK_FINGERPRINT` must be deterministic against the container toolchain. Keeping the `[patch]` in place now stabilizes the manifest for later phases and lets the IT harness depend on `exarrow-rs` with a single shared `arrow = "58"`.
- **Promotes to ADR:** yes

## Review Findings

<!-- Populated by speq-implement after code review. -->
