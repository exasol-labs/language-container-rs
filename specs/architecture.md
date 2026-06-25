# Architecture: lc-rs

How the pure-Rust Script Language Container is put together, how data flows through it,
and the constraints that shape it. For *what* the software must do, see the spec library
(`specs/`); for the project vision, see [`mission.md`](mission.md).

## Overview

```
                 ┌───────────────────────────────────────────────┐
                 │                  EXASOL DB                      │
                 │   (ZMQ ROUTER; drives the whole protocol)       │
                 └───────────────┬───────────────────────────────┘
        localzmq+protobuf control channel │   ▲          ▲
        (ipc:// single-node, tcp:// multi) │   │ MT_*     │ connect-back:
                                           ▼   │          │ separate TCP login
                 ┌─────────────────────────────────────┐ │ to :8563 (Arrow/ADBC)
                 │  exaudfclient (binary)               │ │
                 │  argv: <ipc_socket> lang=rust        │ │
                 │  └── drives a session, exit(0)       │ │
                 └───────────────┬─────────────────────┘ │
                                 ▼                        │
       ┌──────────────────────────────────────────────┐  │
       │  exa-udf-runtime (host)                       │  │
       │  orchestrates: handshake → load → dispatch    │  │
       │                                               │  │
       │   ┌── exa-zmq-protocol ─────────────────────┐ │  │
       │   │  pure state machine (no I/O):           │ │  │
       │   │  ExascriptResponse → HostEvent          │ │  │
       │   │  HostAction → ExascriptRequest          │ │  │
       │   │   └── exa-proto (prost bindings)        │ │  │
       │   └─────────────────────────────────────────┘ │  │
       │                                               │  │
       │   connect-back: OnceLock<tokio current_thread>│──┘
       │   Arrow → Value conversion happens HERE (host)│
       └───────────────┬───────────────────────────────┘
                       │  the ONE ABI crossing:
                       │  extern "C" __exa_udf_entry_<NAME>() -> *const ExaUdfVTable
                       ▼
       ┌──────────────────────────────────────────────┐
       │  user libudf.so  (static musl cdylib)         │
       │   built against exasol-udf-sdk                │
       │   #[exasol_udf] (from exasol-udf-macros)       │
       │   → UdfRun / UdfContext (trait objects        │
       │      live host-side; only Value crosses)      │
       └──────────────────────────────────────────────┘
```

Crate dependency graph (strict acyclic):

```
exaudfclient (binary)
  └── exa-udf-runtime      (host: orchestrates protocol + loader + dispatch)
        ├── exa-zmq-protocol   (pure state machine + ZMQ transport, fully unit-testable without I/O)
        │     └── exa-proto    (prost bindings, no business logic)
        └── exasol-udf-sdk  (ABI types shared with user .so)
              └── exasol-udf-macros  (proc macro, re-exported)
```

## Data flow

- **Lifecycle:** `handshake` (MT_CLIENT/MT_INFO/MT_META, parse the `%udf_object` path) →
  `load` (dlopen the `.so`, resolve `__exa_udf_entry_<NAME>`, validate ABI + fingerprint) →
  `run` loop (MT_RUN; per group: MT_NEXT input batch → UDF `run()` → MT_EMIT output) →
  `cleanup` (MT_FINISHED, then `exit(0)`).
- **Emit path:** `ctx.emit(&row)` pushes into an `EmitBuffer` that tracks a running byte
  estimate and flushes an `MT_EMIT` at the 4,000,000-byte threshold, with a final tail
  flush at end of `run`.
- **Connect-back path (optional):** `ctx.connection("NAME")` fetches CONNECTION-object
  credentials via `MT_IMPORT`, then opens a *separate* SQL login over TCP to `:8563`
  using `exarrow-rs`; reads stream one Arrow batch at a time and are converted to
  `Value` rows host-side before crossing back to the UDF.

## Key design decisions

- `exa-zmq-protocol::Protocol` is a pure state machine (no I/O) that converts
  `ExascriptResponse` → `HostEvent` and `HostAction` → `ExascriptRequest`. The ZMQ socket
  lives only in the transport wrapper — this makes the protocol fully unit-testable with
  fixtures.
- The only ABI crossing is one `extern "C" fn __exa_udf_entry_<NAME>() -> *const ExaUdfVTable`
  per UDF (`<NAME>` is the UPPER_SNAKE_CASE SQL script name); a single `.so` may export
  several. Rich trait objects (`UdfRun`, `UdfContext`) never cross the boundary — they stay
  in the host process; the `sdk_fingerprint` check enforces a matching toolchain at load time.
- Connect-back uses a dedicated `OnceLock<tokio::runtime::Runtime>` (current_thread) so
  async Exasol queries can be called from synchronous UDF code without runtime conflicts.
  The Arrow→`Value` conversion runs on the host side, so only plain `Value` data crosses
  the `.so` boundary.
- `arrow` is pinned at the workspace level in `[workspace.dependencies]` to ensure a single
  copy is shared between `exasol-udf-sdk` and `exarrow-rs`. An Arrow `RecordBatch` cannot
  safely cross the cdylib boundary (`TypeId` is not stable across dynamic libraries), so the
  `ExaConnection` trait is Arrow-free and `emit_batch` ships its `RecordBatch` as Arrow IPC
  bytes that the host re-decodes.

## Project structure

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
└── specs/                      # project spec library
```

## Constraints

- **Technical**: the binary must be at `/exaudf/exaudfclient`; invocation contract is
  `exaudfclient <ipc_socket_path> lang=rust [scriptOptionsParserVersion=1|2]` — must match
  the reference launcher exactly. The Rust toolchain in `rust-toolchain.toml` must equal the
  toolchain baked into the container builder image. `arrow` must stay in sync with `exarrow-rs`.
- **Build**: no Bazel; pure Cargo.
- **Safety**: `catch_unwind` in the `run` shim converts UDF panics to error codes; the failing
  `UdfError`'s message is propagated through an out-pointer into the `F-UDF-CL-RUST-####` close.
  An `sdk_fingerprint` mismatch at `dlopen` must produce a clear error, not UB.
- **Lifecycle**: `main()` must end with `std::process::exit(0)` — a normal return joins the
  connect-back Tokio runtime threads and delays exit, tripping the DB watchdog (SIGABRT).

## External dependencies

| Service | Purpose | Failure Impact |
|---------|---------|----------------|
| Exasol DB (ZMQ ROUTER) | Drives the full protocol; sends input batches and receives output | No UDFs can run |
| `exarrow-rs` | Arrow ADBC connect-back — used exclusively by the host runtime to implement `ExaConnection`; UDFs never link it directly | `ctx.cluster_ip()` / `ctx.connection()` / `ctx.connect_back()` return errors; UDFs that call connect-back fail at runtime |
| BucketFS | Stores precompiled `.so` for Option A | Option A UDFs can't be loaded |
| `zmqcontainer.proto` (GitHub) | Canonical protocol definition for `exa-proto` build | Cannot build `exa-proto`; fetch from the raw GitHub URL (see `PROTO_SOURCES.md`) |

## Exasol data-type mapping

The DB delivers every column over the wire as one of **8 proto column types**
(`exa-proto::ColumnType`). Several SQL types collapse onto the same proto type and are
disambiguated at `ColumnMeta::from_pb` time by inspecting `type_name`. The SDK surfaces the
refined type as `exasol_udf_sdk::value::ExaType` (the single canonical enum; `exa-zmq-protocol`
re-exports it). The detailed scenarios live in `specs/protocol/column-meta`.

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
- **Extended types keep a `String` wire payload** (the proto block does not change); the
  `ExaType` variant — not the `Value` payload — carries the SQL distinction.
- Reference: <https://docs.exasol.com/db/latest/sql_references/data_types/datatypesoverview.htm>

## Connect-back & emit limits

Connect-back reads and `ctx.emit` are both bounded so a UDF cannot OOM the sandbox:

- `ctx.emit` buffers rows and flushes an `MT_EMIT` at a **4,000,000-byte** threshold
  (`EMIT_BUFFER_LIMIT_BYTES`, matching the reference SLC's `SWIG_MAX_VAR_DATASIZE`), with a
  final tail flush at end of `run`. A single row larger than the threshold is still sent as
  one `MT_EMIT`; only the protocol's 2 GB per-value limit remains.
- `ExaConnection::query_for_each` converts the result set one Arrow batch at a time (dropping
  each batch before the next) so the consumer can stream rows; `query` collects via the same
  path for small, bounded results.
