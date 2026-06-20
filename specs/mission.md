# Mission: lc-rs

> A Rust-native Exasol Language Container (SLC) that lets users write Exasol UDFs in Rust by implementing the standard `localzmq+protobuf` wire protocol — with zero dependency on the existing C++ launcher or libexaudflib.

## Problem Statement

Exasol UDFs are currently limited to Python, Java, and R. Teams with Rust codebases or performance-critical compute logic have no path to bring that code into the database. Existing SLCs are implemented in C++ (the launcher) and depend on `libexaudflib_complete.so` loaded via `dlmopen` — a fragile, ABI-sensitive approach that a Rust-native, in-process implementation can replace cleanly.

## Target Users

| Persona | Goal | Key Workflow |
|---------|------|--------------|
| Rust data engineer | Write high-performance UDFs without leaving the Rust ecosystem | Implement the UDF fn, annotate with `#[exasol_udf]`, build a musl `.so`, upload to BucketFS, register in DB |
| Exasol DBA / platform engineer | Deploy and register the Rust SLC in a production cluster | Build + upload the container, `ALTER SESSION SET SCRIPT_LANGUAGES`, create scripts |
| Exasol SDK maintainer | Extend or debug the SLC implementation itself | Run unit tests + integration tests against a local Exasol Docker container |

## Core Capabilities

1. **Full wire-protocol implementation** — handles every `localzmq+protobuf` message type (handshake, scalar, set/EMITS, single-call `SC_FN_*` incl. the virtual-schema adapter call, ping-pong, reset, error close path) so the Rust SLC is indistinguishable from the existing C++ one.
2. **Ergonomic Rust UDF SDK** — exposes the `UdfRun` / `UdfContext` traits and the `#[exasol_udf]` proc macro so authors write idiomatic Rust with typed column access and optional connect-back to Exasol. Connect-back is surfaced via the `ExaConnection` trait (defined in the SDK) so UDFs never link `exarrow-rs` directly — the host process owns the connection; UDFs call `query` / `query_for_each` and receive rows as the SDK's own `Value` type, with the Arrow→`Value` conversion running host-side so Arrow types never cross the `.so` boundary.
3. **Precompiled execution model** — the author builds a fully-static musl `.so` with `cargo exasol-udf build`, uploads it to BucketFS (`exapump bfs upload`), and loads it via a `%udf_object` directive in `CREATE SCRIPT`. (JIT compilation of pasted Rust source — Option C — is not supported in v1; the runtime returns a clear error directing the author to `%udf_object`.)
4. **ABI-safe dynamic loading** — `abi_version` + `sdk_fingerprint` checks at `dlopen` time produce a clear error instead of UB when a `.so` was built with a mismatched toolchain.
5. **Container packaging** — a slim SLC image (no toolchain, precompiled `.so` only), built from `Dockerfile.alpine` (or `Dockerfile.debian`), packaged as a BucketFS tarball and registered with `ALTER SESSION SET SCRIPT_LANGUAGES`. `scripts/install.sh` builds, uploads, and registers it in one step.
6. **Developer tooling** — `cargo-exasol-udf` CLI: scaffold a new UDF crate (`new`) and build a fully-static musl `.so` (`build`; target triple hidden, musl toolchain installed automatically via `rustup`). Pairs with `exapump bfs upload` to push the `.so` to BucketFS, then a `CREATE OR REPLACE RUST SCRIPT … %udf_object` registration. Optional `#[exasol_udf(input(...), emits(...))]` type annotations are validated against the database column metadata at load time.

## Out of Scope

- Python, Java, or R UDF support — handled by existing SLCs.
- Changes to the Exasol core engine or protobuf schema.
- Multi-node UDF fan-out coordination — orchestrated by the DB, transparent to the SLC.
- JIT compilation of pasted Rust source (Option C) — not supported in v1; the runtime returns a clear error directing authors to the precompiled `%udf_object` path.
- Windows or macOS container targets — Linux-only (Alpine / Debian base).

## Domain Glossary

| Term | Definition |
|------|------------|
| SLC | Script Language Container — a Docker image that provides a language runtime for Exasol UDFs, registered via `ALTER SESSION SET SCRIPT_LANGUAGES` |
| UDF | User-Defined Function — a function defined in a script body and executed by the Exasol query engine |
| `localzmq+protobuf` | The IPC wire protocol between the DB and an SLC: ZeroMQ DEALER socket, protobuf-framed messages, one frame per message |
| BucketFS | Exasol's distributed file system; the standard location for uploading precompiled `.so` artifacts |
| Option A | Precompiled-`.so` execution path — author ships a binary, SLC just loads it (the supported path) |
| Option C | JIT execution path — script source compiled in-container on first call. Not supported in v1 (the runtime returns a clear error) |
| ABI fingerprint | `"SDK_VERSION:RUSTC_HASH\0"` string baked into every compiled vtable; guards against toolchain-mismatch UB at load time |
| `ExaConnection` | SDK trait (defined in `exasol-udf-sdk`) exposing `query_arrow`, `query`, `query_for_each`, `execute`, and transaction control (`begin`/`commit`/`rollback`). Host implements it via `exarrow-rs`. UDF code never links `exarrow-rs` directly and uses `query`/`query_for_each`, which return SDK `Value` rows (not Arrow). |
| musl | `x86_64-unknown-linux-musl` target; all Rust deps statically linked; no glibc dependency in the `.so`. `cargo exasol-udf build` targets this automatically. |
| exarrow-rs | crates.io crate (`exarrow-rs` v0.12.7, features = ["websocket"]) providing Arrow-based ADBC connectivity back to Exasol — used by the host runtime only; UDFs access it through the `ExaConnection` trait. |
| `exaudfclient` | The binary the DB invokes per UDF call: `exaudfclient <ipc_socket_path> lang=rust` |
| MT_* | `message_type` enum values in the protobuf protocol (e.g., `MT_RUN`, `MT_NEXT`, `MT_EMIT`) |
| SC_FN_* | Single-call function IDs for import/export spec generation, default output columns, and the virtual-schema adapter call |

---

## Exasol data type mapping

The DB delivers every column over the wire as one of **8 proto column types**
(`exa-proto::ColumnType`). Several SQL types collapse onto the same proto type and
are disambiguated at `ColumnMeta::from_pb` time by inspecting `type_name`. The SDK
surfaces the refined type as `exasol_udf_sdk::value::ExaType` (the single canonical
enum; `exa-zmq-protocol` re-exports it).

| Proto column type | Exasol SQL type(s) | `type_name` disambiguation | SDK `ExaType` | `Value` payload |
|-------------------|--------------------|----------------------------|---------------|-----------------|
| `PB_DOUBLE` | `DOUBLE PRECISION` (`FLOAT`, `REAL`) | none | `Double` | `Double(f64)` |
| `PB_INT32` | `DECIMAL(p,0)` small enough to fit `i32` | none | `Int32` | `Int32(i32)` |
| `PB_INT64` | `DECIMAL(p,0)` fitting `i64` | none | `Int64` | `Int64(i64)` |
| `PB_NUMERIC` | `DECIMAL(p,s)`, `BIGINT`, `NUMBER` | none | `Numeric { precision, scale }` | `Numeric(Decimal)` |
| `PB_DATE` | `DATE` | none | `Date` | `Date(NaiveDate)` |
| `PB_TIMESTAMP` | `TIMESTAMP`, `TIMESTAMP WITH LOCAL TIME ZONE` | `WITH LOCAL TIME ZONE` → `TimestampTz`, else `Timestamp` | `Timestamp` / `TimestampTz` | `Timestamp(NaiveDateTime)` / `String` (TZ) |
| `PB_STRING` | `VARCHAR`, `CHAR`, `GEOMETRY`, `HASHTYPE`, `INTERVAL YEAR TO MONTH`, `INTERVAL DAY TO SECOND` | `CHAR…` → `Char`; `VARCHAR…` → `String`; `GEOMETRY` → `Geometry`; `HASHTYPE` → `HashType`; `INTERVAL…YEAR…MONTH` → `IntervalYearToMonth`; `INTERVAL…DAY…SECOND` → `IntervalDayToSecond` | `String { size }` / `Char { size }` / `Geometry` / `HashType` / `IntervalYearToMonth` / `IntervalDayToSecond` | `String` |
| `PB_BOOLEAN` | `BOOLEAN` | none | `Boolean` | `Bool(bool)` |

Rules:
- **`BIGINT` arrives as `PB_NUMERIC`**, not `PB_INT64`. `get_i64` therefore accepts an
  integral `Value::Numeric`; it errors only on a non-zero fractional part.
- **Only ambiguous proto types consult `type_name`** (`PB_STRING`, `PB_TIMESTAMP`).
  Unambiguous types map directly and MUST NOT read `type_name`.
- **Extended types keep a `String` wire payload** (the proto block does not change);
  the `ExaType` variant — not the `Value` payload — carries the SQL distinction.
- Reference: <https://docs.exasol.com/db/latest/sql_references/data_types/datatypesoverview.htm>

---

## Connect-back limits

Connect-back reads and `ctx.emit` are both bounded so a UDF cannot OOM the sandbox:

- `ctx.emit` buffers rows and flushes an `MT_EMIT` at a **4,000,000-byte** threshold
  (`EMIT_BUFFER_LIMIT_BYTES`, matching the C++ SLC's `SWIG_MAX_VAR_DATASIZE`), with a
  final tail flush at end of `run`. A single row larger than the threshold is still
  sent as one `MT_EMIT`; only the protocol's 2 GB per-value limit remains.
- `ExaConnection::query_for_each` converts the result set one Arrow batch at a time
  (dropping each batch before the next) so the consumer can stream rows; `query`
  collects via the same path for small, bounded results.

---

## Tech Stack

| Layer | Technology | Purpose |
|-------|------------|---------|
| Language | Rust (pinned 1.92 — see `rust-toolchain.toml`) | All crates |
| ZMQ | `zmq = "0.10"` (libzmq C bindings) | DEALER socket transport |
| Protobuf | `prost = "0.13"`, `prost-build` | Code-gen from `zmqcontainer.proto`; no runtime `protoc` |
| Dynamic loading | `libloading = "0.8"` | `dlopen` user `libudf.so` |
| Arrow | `arrow = "58"` | Host-side connect-back batch decoding; pinned to match `exarrow-rs` |
| DB connect-back | `exarrow-rs` (crates.io, v0.12.7) | Arrow ADBC connection back to Exasol from UDF code |
| Async | `tokio = "1"` (current_thread) | Connect-back only; never enters the ZMQ loop |
| Proc macro | `syn = "2"`, `quote = "1"`, `proc-macro2` | `#[exasol_udf]` attribute macro |
| Errors | `thiserror`, `anyhow` (binary only) | Typed and ad-hoc error handling |
| Logging | `tracing`, `tracing-subscriber` | Stderr only; Exasol captures stderr as UDF log |
| Testing | `cargo test`, `trybuild`, `testcontainers` | Unit, integration (live Docker DB), compile-fail tests |
| Container base | Alpine (`Dockerfile.alpine`) / Debian (`Dockerfile.debian`); builder `rust:1.92-bookworm` | Runtime image |

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

## Project Structure

```
lc-rs/
├── Cargo.toml                  # workspace root; exarrow-rs + arrow pinned in [workspace.dependencies]
├── rust-toolchain.toml         # pinned Rust channel — MUST match the container builder
├── Cargo.lock
├── Dockerfile.alpine           # slim SLC image (alpine:3 runtime) — built by CI + scripts/install.sh
├── Dockerfile.debian           # slim SLC image (debian:12-slim runtime) variant
├── crates/
│   ├── exa-proto/              # prost-generated protobuf bindings
│   ├── exa-zmq-protocol/       # ZMQ DEALER transport + typed state machine
│   ├── exasol-udf-sdk/         # PUBLIC: UdfRun/UdfContext traits, Value/ExaType, connect-back
│   ├── exasol-udf-macros/      # PUBLIC: proc-macro #[exasol_udf]
│   ├── exa-udf-runtime/        # host: loads .so, drives protocol ↔ SDK
│   ├── exaudfclient/           # binary: /exaudf/exaudfclient
│   ├── cargo-exasol-udf/       # PUBLIC: CLI — scaffold + build musl .so
│   └── it/                     # integration tests (live Exasol Docker, `--features integration`)
├── test-udfs/                  # example/fixture UDF crates exercised by the integration tests
├── scripts/                    # install.sh (build + upload + register), ci-it-local.sh
├── docs/                       # user-facing docs (installation, writing-a-udf, protocol, cargo-ecosystem)
└── specs/                      # project spec library (this directory)
```

## Architecture

Layered pipeline with a strict acyclic dependency graph:

```
exaudfclient (binary)
  └── exa-udf-runtime      (host: orchestrates protocol + loader + dispatch)
        ├── exa-zmq-protocol   (pure state machine + ZMQ transport, fully unit-testable without I/O)
        │     └── exa-proto    (prost bindings, no business logic)
        └── exasol-udf-sdk  (ABI types shared with user .so)
              └── exasol-udf-macros  (proc macro, re-exported)
```

**Key design decisions:**
- `exa-zmq-protocol::Protocol` is a pure state machine (no I/O) that converts `ExascriptResponse` → `HostEvent` and `HostAction` → `ExascriptRequest`. The ZMQ socket lives only in the transport wrapper — this makes the protocol fully unit-testable with fixtures.
- The only ABI crossing is one `extern "C" fn __exa_udf_entry_<NAME>() -> *const ExaUdfVTable` per UDF (`<NAME>` is the UPPER_SNAKE_CASE SQL script name); a single `.so` may export several. Rich trait objects (`UdfRun`, `UdfContext`) never cross the boundary — they stay in the host process; the `sdk_fingerprint` check enforces a matching toolchain at load time.
- Connect-back uses a dedicated `OnceLock<tokio::runtime::Runtime>` (current_thread) so async Exasol queries can be called from synchronous UDF code without runtime conflicts. The Arrow→`Value` conversion runs on the host side, so only plain `Value` data crosses the `.so` boundary.
- Arrow `= "58"` is pinned at the workspace level in `[workspace.dependencies]` to ensure a single copy is shared between `exasol-udf-sdk` and `exarrow-rs`.

## Constraints

- **Technical**: Binary must be at `/exaudf/exaudfclient`; invocation contract is `exaudfclient <ipc_socket_path> lang=rust [scriptOptionsParserVersion=1|2]` — must match the C++ launcher exactly. The Rust toolchain in `rust-toolchain.toml` must equal the toolchain baked into the container builder image. `arrow = "58"` must stay in sync with `exarrow-rs`.
- **Build**: No Bazel; pure Cargo.
- **Safety**: `catch_unwind` in the `run` shim converts UDF panics to error codes; the failing `UdfError`'s message is propagated through an out-pointer into the `F-UDF-CL-RUST-####` close. An `sdk_fingerprint` mismatch at `dlopen` must produce a clear error, not UB.
- **Lifecycle**: `main()` must end with `std::process::exit(0)` — a normal return joins the connect-back Tokio runtime threads and delays exit, tripping the DB watchdog (SIGABRT).

## External Dependencies

| Service | Purpose | Failure Impact |
|---------|---------|----------------|
| Exasol DB (ZMQ ROUTER) | Drives the full protocol; sends input batches and receives output | No UDFs can run |
| `exarrow-rs` (crates.io, v0.12.7) | Arrow ADBC connect-back — used exclusively by the host runtime to implement `ExaConnection`; UDFs never link it directly | `ctx.cluster_ip()` / `ctx.connection()` / `ctx.connect_back()` return errors; UDFs that call connect-back fail at runtime |
| BucketFS | Stores precompiled `.so` for Option A | Option A UDFs can't be loaded |
| `zmqcontainer.proto` (GitHub / git submodule) | Canonical protocol definition for `exa-proto` build | Cannot build `exa-proto`; fetch from the raw GitHub URL (see `PROTO_SOURCES.md`) |

---

## References

- User-facing documentation lives in [`docs/`](../docs/index.md): installation, writing a UDF, the wire protocol, and the cargo ecosystem.
- Architectural decisions are recorded in [`decision-log.md`](decision-log.md); deferred work in [`backlog.md`](backlog.md).
