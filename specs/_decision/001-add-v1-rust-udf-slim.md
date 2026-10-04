# Decisions: add-v1-rust-udf-slim

## ADR: v1 uses Option A (precompiled .so) only; JIT returns unsupported

**ID:** v1-option-a-precompiled-so-only
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

## ADR: Integration tests use testcontainers-rs with a pinned DB image in privileged mode

**ID:** testcontainers-privileged-db-image
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

## ADR: BucketFS upload via HTTP PUT and SQL via exarrow-rs directly

**ID:** bucketfs-upload-http-put-sql-exarrow-rs
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

## ADR: Pure I/O-free protocol state machine separated from ZMQ transport

**ID:** pure-io-free-protocol-state-machine
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

## ADR: Single C-ABI crossing with ABI-version and fingerprint gating

**ID:** single-c-abi-crossing-abi-version-fingerprint
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
