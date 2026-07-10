# Mission: lc-rs

> Bring Rust into Exasol UDFs — write high-performance, memory-safe user-defined
> functions as idiomatic Rust, compile them to a single static `.so`, and run them in
> a pure-Rust Script Language Container.

## Value

`lc-rs` lets teams who already build in Rust extend Exasol with their own compute logic:
typed column access, zero-cost abstractions, and the whole crates.io ecosystem, all
inside the database. UDFs are precompiled to a fully-static musl `.so` and loaded by a
slim container, so there is no in-container toolchain and no runtime compilation —
deploy is "upload the `.so`, register the script." Authors can optionally connect back
to Exasol for reference data, and stream large result sets and emit batches without
exhausting the UDF sandbox.

## Target Users

| Persona | Goal | Key Workflow |
|---------|------|--------------|
| Rust data engineer | Write high-performance UDFs without leaving the Rust ecosystem | Implement the UDF fn, annotate with `#[exasol_udf]`, build a musl `.so`, upload to BucketFS, register in DB |
| Exasol DBA / platform engineer | Deploy and register the Rust SLC in a production cluster | Build + upload the container, `ALTER SESSION SET SCRIPT_LANGUAGES`, create scripts |
| Exasol SDK maintainer | Extend or debug the SLC implementation itself | Run unit tests + integration tests against a local Exasol Docker container |

## Core Capabilities

1. **Full wire-protocol implementation** — handles every `localzmq+protobuf` message type (handshake, scalar, set/EMITS, single-call `SC_FN_*` incl. the virtual-schema adapter call, ping-pong, reset, error close).
2. **Ergonomic Rust UDF SDK** — the `UdfRun` / `UdfContext` traits plus the `#[exasol_udf]` proc macro give typed column access and optional connect-back, with rows surfaced as the SDK's own `Value` type.
3. **Precompiled execution model** — build a static musl `.so` with `cargo exasol-udf build`, upload to BucketFS, load via a `%udf_object` directive in `CREATE SCRIPT`.
4. **ABI-safe dynamic loading** — `abi_version` + `sdk_fingerprint` checks at load time turn a toolchain mismatch into a clear error instead of UB.
5. **Container packaging** — a slim SLC image (no toolchain, precompiled `.so` only), packaged as a BucketFS tarball and registered with `ALTER SESSION SET SCRIPT_LANGUAGES`; `scripts/install.sh` builds, uploads, and registers in one step. The image ships a generated third-party license/attribution bundle (`cargo-about`-generated OS package notices, copied glibc/GCC runtime licenses, GPL-3.0 written-source offer) alongside the runtime.
6. **Developer tooling** — the `cargo-exasol-udf` CLI scaffolds a UDF crate (`new`), builds the static musl `.so` (`build`), and validates the ABI of a built artifact (`validate`).
7. **Live diagnostics** — a `%udf_debug_level` script directive tunes runtime tracing verbosity, exposes an SDK `log` surface for UDF-authored lines, and reports memory/emit-buffer telemetry at debug level, all carried over Exasol's `SET SESSION SCRIPT OUTPUT ADDRESS` stderr redirect.

> Detailed behavior lives in the spec library (`specs/sdk`, `specs/protocol`,
> `specs/runtime`, `specs/tools`, `specs/container`, `specs/binary`, `specs/examples`).
> Design, structure, and the data-type mapping are in [`architecture.md`](architecture.md).

## Domain Glossary

| Term | Definition |
|------|------------|
| SLC | Script Language Container — a Docker image that provides a language runtime for Exasol UDFs, registered via `ALTER SESSION SET SCRIPT_LANGUAGES` |
| UDF | User-Defined Function — a function defined in a script body and executed by the Exasol query engine |
| `localzmq+protobuf` | The IPC wire protocol between the DB and an SLC: the client opens a ZeroMQ REQ socket to the DB's REP socket, protobuf-framed messages, one frame per message |
| BucketFS | Exasol's distributed file system; the standard location for uploading precompiled `.so` artifacts |
| Option A | Precompiled-`.so` execution path — author ships a binary, SLC just loads it (the supported path) |
| Option C | JIT execution path — script source compiled in-container on first call. Not supported (the runtime returns a clear error) |
| ABI fingerprint | `"SDK_VERSION:RUSTC_HASH\0"` string baked into every compiled vtable; guards against toolchain-mismatch UB at load time |
| `ExaConnection` | SDK trait (defined in `exasol-udf-sdk`) exposing `query`, `query_for_each`, `execute`, and transaction control. Host implements it via `exarrow-rs`; UDF code never links `exarrow-rs` directly and receives SDK `Value` rows (not Arrow). |
| musl | `x86_64-unknown-linux-musl` target; all Rust deps statically linked; no glibc dependency in the `.so`. `cargo exasol-udf build` targets this automatically. |
| exarrow-rs | crates.io crate providing Arrow-based ADBC connectivity back to Exasol — used by the host runtime only; UDFs access it through the `ExaConnection` trait. |
| `exaudfclient` | The binary the DB invokes per UDF call: `exaudfclient <ipc_socket_path> lang=rust` |
| MT_* | `message_type` enum values in the protobuf protocol (e.g., `MT_RUN`, `MT_NEXT`, `MT_EMIT`) |
| SC_FN_* | Single-call function IDs for import/export spec generation, default output columns, and the virtual-schema adapter call |

## Tech Stack

| Layer | Technology | Purpose |
|-------|------------|---------|
| Language | Rust (channel pinned in `rust-toolchain.toml`) | All crates |
| ZMQ | `zmq` (libzmq C bindings) | REQ socket transport (client REQ ↔ DB REP) |
| Protobuf | `prost`, `prost-build` | Code-gen from `zmqcontainer.proto`; no runtime `protoc` |
| Dynamic loading | `libloading` | `dlopen` user `libudf.so` |
| Arrow | `arrow` (pinned to match `exarrow-rs`) | Host-side connect-back batch decoding |
| DB connect-back | `exarrow-rs` | Arrow ADBC connection back to Exasol from UDF code |
| Async | `tokio` (current_thread) | Connect-back only; never enters the ZMQ loop |
| Proc macro | `syn`, `quote`, `proc-macro2` | `#[exasol_udf]` attribute macro |
| Errors | `thiserror`, `anyhow` (binary only) | Typed and ad-hoc error handling |
| Logging | `tracing`, `tracing-subscriber` | Stderr only; Exasol captures stderr as UDF log |
| Testing | `cargo test`, `trybuild`, `testcontainers` | Unit, integration (live Docker DB), compile-fail tests |
| Container base | Alpine (`Dockerfile.alpine`) / Debian (`Dockerfile.debian`) | Runtime image |

## Commands

```bash
# Build the exaudfclient binary
cargo build --release -p exaudfclient

# Build everything
cargo build --release

# Run all unit tests (the live-DB integration crate is excluded by default)
cargo test

# Run integration tests against a local Exasol Docker container
cargo test -p it --features integration

# Lint & format check
cargo fmt --check && cargo clippy --all-targets --all-features -- -D warnings

# Scaffold a new UDF crate
cargo exasol-udf new my-udf

# Build a fully-static musl UDF .so (musl toolchain installed automatically)
cargo exasol-udf build
# → target/x86_64-unknown-linux-musl/release/libmy_udf.so

# Deploy: upload to BucketFS, then register the script
exapump bfs upload \
  target/x86_64-unknown-linux-musl/release/libmy_udf.so \
  /buckets/bfsdefault/default/udf/libmy_udf.so
# then CREATE OR REPLACE RUST … SCRIPT … AS %udf_object /buckets/…/libmy_udf.so;
```

## References

- Architecture, project structure, and the Exasol data-type mapping: [`architecture.md`](architecture.md).
- User-facing documentation: [`docs/`](../docs/index.md) — installation, writing a UDF, the wire protocol, the cargo ecosystem.
- Architectural decisions: [`decision-log.md`](decision-log.md).
