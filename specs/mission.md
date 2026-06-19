# Mission: lc-rs

> A Rust-native Exasol Language Container (SLC) that lets users write Exasol UDFs in Rust by implementing the standard `localzmq+protobuf` wire protocol — with zero dependency on the existing C++ launcher or libexaudflib.

## Problem Statement

Exasol UDFs are currently limited to Python, Java, and R. Teams with Rust codebases or performance-critical compute logic have no path to bring that code into the database. Existing SLCs are implemented in C++ (the launcher) and depend on `libexaudflib_complete.so` loaded via `dlmopen` — a fragile, ABI-sensitive approach that a Rust-native, in-process implementation can replace cleanly.

## Target Users

| Persona | Goal | Key Workflow |
|---------|------|--------------|
| Rust data engineer | Write high-performance UDFs without leaving the Rust ecosystem | Implement `UdfRun`, annotate with `#[exasol_udf]`, build `.so` or paste source, register in DB |
| Exasol DBA / platform engineer | Deploy and register the Rust SLC in a production cluster | Upload container, `ALTER SESSION SET SCRIPT_LANGUAGES`, create scripts |
| Exasol SDK maintainer | Extend or debug the SLC implementation itself | Run integration tests against the emulator or a real DB |

## Core Capabilities

1. **Full wire-protocol implementation** — handles every `localzmq+protobuf` message type (handshake, scalar, set/EMITS, single-call SC_FN_*, ping-pong, reset, error close path) so the Rust SLC is indistinguishable from the existing C++ one.
2. **Ergonomic Rust UDF SDK** — exposes `UdfRun` / `UdfContext` traits and `#[exasol_udf]` proc macro so authors write idiomatic Rust with typed column access, Arrow batch fast paths, and optional connect-back to Exasol. Connect-back is surfaced via the `ExaConnection` trait (defined in the SDK) so UDFs never link `exarrow-rs` directly — the host process owns the connection, the UDF just calls trait methods returning Arrow `RecordBatch`.
3. **Dual execution model** — Precompiled mode (primary author path): author builds a fully-static musl `.so` with `cargo exasol-udf build`, uploads via `exapump udf deploy`, SLC loads it directly from BucketFS. JIT mode (secondary): Rust source pasted into `CREATE SCRIPT` body, compiled in-container on first call and cached.
4. **ABI-safe dynamic loading** — `abi_version` + `sdk_fingerprint` checks at `dlopen` time produce a clear error instead of UB when a `.so` was built with a mismatched toolchain.
5. **Container packaging** — `slim` image (~150 MB, no toolchain, precompiled `.so` only) and `jit` image (~1.4 GB, full Rust toolchain + vendored Cargo registry), both plugging into the `exaslct` flavor DAG.
6. **Developer tooling** — `cargo-exasol-udf` CLI: scaffold a new UDF crate, build a fully-static musl `.so` (target triple hidden, musl toolchain installed automatically via `rustup`), and validate ABI compatibility locally. Pairs with `exapump udf deploy` for one-command upload + `CREATE OR REPLACE RUST SCRIPT` generation (type annotations from `#[exasol_udf(input(...), emits(...))]` are optional but used automatically when present).

## Out of Scope

- Python, Java, or R UDF support — handled by existing SLCs.
- Changes to the Exasol core engine or protobuf schema.
- Multi-node UDF fan-out coordination — orchestrated by the DB, transparent to the SLC.
- Virtual schema adapter calls (`SC_FN_VIRTUAL_SCHEMA_ADAPTER_CALL`) — stubbed as `MT_UNDEFINED_CALL` in v1.
- Windows or macOS container targets — Linux-only (`debian:12-slim` base).

## Domain Glossary

| Term | Definition |
|------|------------|
| SLC | Script Language Container — a Docker image that provides a language runtime for Exasol UDFs, registered via `ALTER SESSION SET SCRIPT_LANGUAGES` |
| UDF | User-Defined Function — a function defined in a script body and executed by the Exasol query engine |
| `localzmq+protobuf` | The IPC wire protocol between the DB and an SLC: ZeroMQ DEALER socket, protobuf-framed messages, one frame per message |
| BucketFS | Exasol's distributed file system; the standard location for uploading precompiled `.so` artifacts |
| Option A | Precompiled-`.so` execution path — author ships a binary, SLC just loads it |
| Option C | JIT execution path — script source is compiled inside the container on first call |
| ABI fingerprint | `"SDK_VERSION:RUSTC_HASH\0"` string baked into every compiled vtable; guards against toolchain-mismatch UB at load time |
| `ExaConnection` | SDK trait (defined in `exasol-udf-sdk`) exposing `query_arrow`, `execute`, `import_arrow`, `export_arrow`. Host implements it via `exarrow-rs`. UDF code never links `exarrow-rs` directly. |
| musl | `x86_64-unknown-linux-musl` target; all Rust deps statically linked; no glibc dependency in the `.so`. `cargo exasol-udf build` targets this automatically. |
| exarrow-rs | crates.io crate (`exarrow-rs` v0.12.7, features = ["websocket"]) providing Arrow-based ADBC connectivity back to Exasol — used by the host runtime only; UDFs access it through the `ExaConnection` trait. |
| exaslct | Exasol's SLC build-and-release toolchain; the CI pipeline that assembles, tests, and publishes SLC images |
| `exaudfclient` | The binary the DB invokes per UDF call: `exaudfclient <ipc_socket_path> lang=rust` |
| MT_* | `message_type` enum values in the protobuf protocol (e.g., `MT_RUN`, `MT_NEXT`, `MT_EMIT`) |
| SC_FN_* | Single-call function IDs for import/export spec generation and default output columns |

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

## Tech Stack

| Layer | Technology | Purpose |
|-------|------------|---------|
| Language | Rust (stable, pinned — see `rust-toolchain.toml`) | All crates |
| ZMQ | `zmq = "0.10"` (libzmq C bindings) | DEALER socket transport |
| Protobuf | `prost = "0.13"`, `prost-build` | Code-gen from `zmqcontainer.proto`; no runtime `protoc` |
| Dynamic loading | `libloading = "0.8"` | `dlopen` user `libudf.so` |
| Arrow | `arrow = "58"` | Zero-copy batch I/O; pinned to match `exarrow-rs` |
| DB connect-back | `exarrow-rs` (crates.io, v0.12.7) | Arrow ADBC connection back to Exasol from UDF code |
| Async | `tokio = "1"` (current_thread) | Connect-back only; never enters the ZMQ loop |
| Proc macro | `syn = "2"`, `quote = "1"`, `proc-macro2` | `#[exasol_udf]` attribute macro |
| Errors | `thiserror`, `anyhow` (binary only) | Typed and ad-hoc error handling |
| Logging | `tracing`, `tracing-subscriber` | Stderr only; Exasol captures stderr as UDF log |
| Testing | `cargo test`, `trybuild` | Unit, integration, compile-fail tests |
| Container base | `debian:12-slim` | Runtime image |

## Commands

```bash
# Build the exaudfclient binary
cargo build --release -p exaudfclient

# Build everything
cargo build --release

# Run all tests
cargo test

# Lint & format check
cargo fmt --check && cargo clippy --all-targets --all-features -- -D warnings

# Format
cargo fmt

# Scaffold a new UDF crate
cargo exasol-udf new my-udf

# Build a fully-static musl UDF .so (musl toolchain installed automatically)
cargo exasol-udf build
# → target/x86_64-unknown-linux-musl/release/libmy_udf.so

# Deploy: upload to BucketFS + CREATE OR REPLACE RUST SCRIPT
exapump udf deploy \
  --so target/x86_64-unknown-linux-musl/release/libmy_udf.so \
  --bucket bfsdefault/default/udfs/ \
  --script my_schema.my_udf \
  --inputs "x BIGINT" --outputs "result BIGINT"
# If #[exasol_udf(input(...), emits(...))] is annotated, --inputs/--outputs are inferred automatically
```

## Project Structure

```
lc-rs/
├── Cargo.toml                    # workspace root; exarrow-rs + arrow pinned in [workspace.dependencies]
├── rust-toolchain.toml           # pinned Rust channel — MUST match container
├── Cargo.lock
├── crates/
│   ├── exa-proto/                # prost-generated protobuf bindings
│   ├── exa-zmq-protocol/         # ZMQ DEALER transport + typed state machine
│   ├── exasol-udf-sdk/           # PUBLIC: UdfRun/UdfContext traits, Value, Arrow bridge
│   ├── exasol-udf-macros/        # proc-macro: #[exasol_udf]
│   ├── exa-udf-runtime/          # host: loads .so, drives protocol ↔ SDK
│   ├── exaudfclient/             # binary: /exaudf/exaudfclient
│   └── cargo-exasol-udf/             # CLI: scaffold + build + validate UDF .so locally
├── container/
│   ├── template-crate/           # pre-baked crate template for JIT builds
│   └── cargo-offline.toml        # .cargo/config.toml pointing to /opt/lc-rs/vendor
└── specs/                        # project spec library (this directory)
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
- The only ABI crossing is `extern "C" fn __exa_udf_entry() -> *const ExaUdfVTable`. Rich trait objects (`UdfRun`, `UdfContext`) never cross the boundary — they stay in the host process. For Option C, the same compiler guarantees this; for Option A, the `sdk_fingerprint` check enforces it.
- Connect-back uses a dedicated `OnceLock<tokio::runtime::Runtime>` (current_thread) so async Exasol queries can be called from synchronous UDF code without runtime conflicts.
- Arrow `= "58"` is pinned at the workspace level in `[workspace.dependencies]` to ensure a single copy is shared between `exasol-udf-sdk` and `exarrow-rs`, enabling zero-copy `RecordBatch` pass-through.

## Constraints

- **Technical**: Binary must be at `/exaudf/exaudfclient`; invocation contract is `exaudfclient <ipc_socket_path> lang=rust [scriptOptionsParserVersion=1|2]` — must match the C++ launcher exactly. Rust toolchain version in `rust-toolchain.toml` must equal the toolchain baked into the `jit` container image. `arrow = "58"` must stay in sync with `exarrow-rs`.
- **Build**: No Bazel; pure Cargo. JIT builds run `cargo build --offline` inside the container against the vendored registry at `/opt/lc-rs/vendor/`.
- **Performance**: JIT warm-cache (keyed by `sha256(source ++ SDK_VERSION ++ RUSTC_VERSION)`) must skip compilation entirely. Arrow batch fast path (`get_arrow_batch` / `emit_arrow_batch`) must avoid row-by-row allocation.
- **Safety**: `catch_unwind` in the `run` shim converts UDF panics to error codes. `sdk_fingerprint` mismatch at `dlopen` must produce a clear error, not UB.

## External Dependencies

| Service | Purpose | Failure Impact |
|---------|---------|----------------|
| Exasol DB (ZMQ ROUTER) | Drives the full protocol; sends input batches and receives output | No UDFs can run |
| `exarrow-rs` (crates.io, v0.12.7) | Arrow ADBC connect-back — used exclusively by the host runtime to implement `ExaConnection`; UDFs never link it directly | `ctx.exa()` / `ctx.exa_named()` / `ctx.exa_connect()` return errors; UDFs that call connect-back fail at runtime |
| BucketFS | Stores precompiled `.so` for Option A | Option A UDFs can't be loaded; JIT unaffected |
| Vendored Cargo registry (`/opt/lc-rs/vendor/`) | Enables offline JIT builds inside the container | JIT compilation fails; Option A unaffected |
| `zmqcontainer.proto` (GitHub / git submodule) | Canonical protocol definition for `exa-proto` build | Cannot build `exa-proto`; fetch via `git submodule update --init` in `script-languages-release` or from the raw GitHub URL |
| `exaslct` build pipeline | Assembles, tests, and publishes the final SLC Docker images | Container images cannot be released |

---

## References

- [Technical Design](references/design.md) — full wire-protocol spec, crate structure, Dockerfile stages, testing strategy, known gaps
