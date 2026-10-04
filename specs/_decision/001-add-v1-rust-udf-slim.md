# Decisions: add-v1-rust-udf-slim

## ADR: v1 uses Option A (precompiled .so) only; JIT returns unsupported

**ID:** v1-option-a-precompiled-so-only
**Plan:** `add-v1-rust-udf-slim`
**Status:** Accepted

### Context

JIT compilation (Option C) needs a ~1.4 GB image with a vendored Cargo registry and an in-container compile/cache pipeline. Precompiled `.so` loading (Option A) covers the full protocol, SDK, loader and dispatch surface without that infrastructure.

### Decision

The runtime executes only precompiled `.so` artifacts (Option A). The compiler entry point returns an unsupported-feature error for the JIT path (Option C).

### Options Considered

| Option | Verdict |
|--------|---------|
| Option A only (precompiled .so) | ✓ Chosen, least infrastructure |
| Option C (JIT) also | ✗ Needs the ~1.4 GB image, vendored registry and in-container pipeline |

### Consequences

The slim image supports only `.so` artifacts uploaded to BucketFS. `compiler.rs` returns an explicit unsupported error.

## ADR: Integration tests use testcontainers-rs with a pinned DB image in privileged mode

**ID:** testcontainers-privileged-db-image
**Plan:** `add-v1-rust-udf-slim`
**Status:** Accepted

### Context

The integration tests must prove the BucketFS upload, `ALTER SESSION`, `CREATE SCRIPT` and `SELECT` path against a real Exasol database.

### Decision

Integration tests use `testcontainers-rs` to start `exasol/docker-db:2026.1.0` with `with_privileged(true)`, exposing DB port `8563` and BucketFS port `2580`. Tests are gated behind an `integration` Cargo feature.

### Options Considered

| Option | Verdict |
|--------|---------|
| testcontainers-rs, pinned image, privileged | ✓ Chosen, self-contained with RAII teardown |
| Manual docker-compose harness | ✗ Brittle lifecycle, harder to gate in CI |
| Script-languages emulator | ✗ Cannot exercise BucketFS upload, `ALTER SESSION`, `CREATE SCRIPT` or `SELECT` |

### Consequences

Running the integration tests requires Docker with privileged-container support. The `integration` feature keeps default `cargo test` Docker-free.

## ADR: BucketFS upload via HTTP PUT and SQL via exarrow-rs directly

**ID:** bucketfs-upload-http-put-sql-exarrow-rs
**Plan:** `add-v1-rust-udf-slim`
**Status:** Accepted

### Context

The integration harness must upload `.so` artifacts to BucketFS and run SQL assertions.

### Decision

The harness uploads BucketFS artifacts with `reqwest` HTTP PUT to `http://w:<write-password>@<host>:<bucketfs-port>/<bucket>/<path>` and runs all SQL through `exarrow-rs` with `validate_server_certificate(false)`.

### Options Considered

| Option | Verdict |
|--------|---------|
| HTTP PUT (reqwest) + exarrow-rs | ✓ Chosen, callable from the test crate |
| Shell out to exapump | ✗ CLI, not a library; adds a process dependency and output parsing |

### Consequences

The `it` crate takes `reqwest` and `exarrow-rs` as dev-dependencies. Certificate validation is disabled per project rules (`validateservercertificate=0`).

## ADR: Pure I/O-free protocol state machine separated from ZMQ transport

**ID:** pure-io-free-protocol-state-machine
**Plan:** `add-v1-rust-udf-slim`
**Status:** Accepted

### Context

The protocol state machine handles more than a dozen message types and phase transitions. Socket I/O inside it would make it untestable without libzmq.

### Decision

`exa-zmq-protocol::Protocol` consumes decoded `ExascriptResponse` values and produces `ExascriptRequest`/`HostEvent` values with no socket I/O. The DEALER socket lives only in `ZmqTransport`.

### Options Considered

| Option | Verdict |
|--------|---------|
| Pure state machine, I/O in ZmqTransport | ✓ Chosen, unit-testable with fixtures |
| Socket I/O inside the state machine | ✗ Message-ordering logic not deterministically testable |

### Consequences

Only `ZmqTransport` needs integration-level tests.

## ADR: Single C-ABI crossing with ABI-version and fingerprint gating

**ID:** single-c-abi-crossing-abi-version-fingerprint
**Plan:** `add-v1-rust-udf-slim`
**Status:** Accepted

### Context

Rust has no stable ABI, so rich trait objects across a `dlopen` boundary risk undefined behavior. The loader must reject toolchain mismatches before they cause it.

### Decision

The only FFI boundary is `extern "C" fn __exa_udf_entry() -> *const ExaUdfVTable`. The loader checks `abi_version == 1` and the `sdk_fingerprint` before calling `create`. The `#[exasol_udf]` macro embeds a `build.rs`-baked fingerprint and wraps `run` in `catch_unwind`.

### Options Considered

| Option | Verdict |
|--------|---------|
| Single `#[repr(C)]` vtable with abi_version and fingerprint | ✓ Chosen |
| Rich trait objects across the boundary | ✗ No stable ABI, undefined behavior |
| No fingerprint check | ✗ Toolchain mismatch becomes silent undefined behavior |

### Consequences

A `.so` built with a mismatched toolchain or SDK is rejected at load time. A panic in UDF code becomes an error code instead of unwinding across FFI.
