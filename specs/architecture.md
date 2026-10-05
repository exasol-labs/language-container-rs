# Architecture

## Overview

```
┌──────────────────────────────────────────────┐
│ Exasol DB (ZMQ REP socket, drives protocol)  │
└──────────────────────────────────────────────┘
          ▲  localzmq+protobuf, REQ/REP lockstep
          │  (ipc:// single node, tcp:// multi node)
          ▼
┌──────────────────────────────────────────────┐
│ exaudfclient (binary)                        │
│ argv: <endpoint> lang=rust  ->  exit(code)   │
└──────────────────────────────────────────────┘
          │
          ▼
┌──────────────────────────────────────────────┐
│ exa-udf-runtime (host)                       │
│ handshake -> load -> validate -> dispatch    │
│   -> cleanup -> MT_FINISHED                  │
│  · exa-zmq-protocol: Protocol state machine  │
│    + ZmqTransport (REQ socket)               │
│     └ exa-proto: prost bindings              │
│  · connect-back: current_thread Tokio +      │
│    exarrow-rs, Arrow -> Value host-side      │
└──────────────────────────────────────────────┘
          │  one C ABI symbol per UDF:
          │  __exa_udf_entry_<NAME>() -> *const ExaUdfVTable
          ▼
┌──────────────────────────────────────────────┐
│ user libudf.so (glibc cdylib)                │
│ #[exasol_udf] -> UdfRun / UdfContext         │
│ data crosses as Value rows or Arrow IPC      │
└──────────────────────────────────────────────┘

Connect-back (optional): a separate SQL session the host
opens over TCP via exarrow-rs, independent of the ZMQ channel.
```

```
exaudfclient
  └── exa-udf-runtime
        ├── exa-zmq-protocol
        │     ├── exa-proto
        │     └── exasol-udf-sdk
        ├── exa-proto
        └── exasol-udf-sdk

user UDF crate (cdylib)
  ├── exasol-udf-sdk
  └── exasol-udf-macros
        └── exasol-udf-sdk
```

- Host process plus dynamically loaded plugin: one `exaudfclient` process per UDF VM speaks the Exasol wire protocol and calls the precompiled UDF `.so` through a single `repr(C)` vtable.
- The workspace is a pure Cargo workspace with an acyclic crate graph. Shared dependency versions live in `[workspace.dependencies]`.

## Components

- exaudfclient (crates/exaudfclient/): process entry point; validates argv, installs the stderr tracing subscriber with a reloadable level filter, runs the runtime, and ends with `std::process::exit` | owns: process exit code, tracing filter handle | depends on: exa-udf-runtime
- exa-udf-runtime (crates/exa-udf-runtime/): host runtime; drives one UDF session through handshake, `%udf_object` resolution, `.so` load, output-shape and annotated-schema checks, scalar/set or single-call dispatch, cleanup hook, and the final message | owns: loaded UDF handle, emit buffer, input row set, connect-back Tokio runtime | depends on: exa-zmq-protocol, exa-proto, exasol-udf-sdk, exarrow-rs
- exa-zmq-protocol (crates/exa-zmq-protocol/): typed protocol state machine (`Protocol` maps each `ExascriptResponse` to a `HostEvent` and `HostAction`), the ZMQ REQ transport, and the proto-to-`ExaType` column mapping | owns: protocol phase, `UdfMeta`, `ConnInfo` | depends on: exa-proto, exasol-udf-sdk
- exa-proto (crates/exa-proto/): prost-generated bindings for the vendored `zmqcontainer.proto`, compiled with a vendored `protoc` | owns: wire message types | depends on: none
- exasol-udf-sdk (crates/exasol-udf-sdk/): public SDK; `UdfRun` and `UdfContext` traits, `Value`, `ExaType`, `ColumnInfo`, `ExaConnection`, the ABI vtable and fingerprint, the `udf_log!` macro, and an optional Arrow `RecordBatch` emit path | owns: ABI version, SDK fingerprint | depends on: none
- exasol-udf-macros (crates/exasol-udf-macros/): public proc macro `#[exasol_udf]`; generates the `__exa_udf_entry_<NAME>` symbol and vtable, wraps calls in `catch_unwind`, and embeds annotated schemas and the output shape | owns: generated vtable code | depends on: exasol-udf-sdk
- cargo-exasol-udf (crates/cargo-exasol-udf/): public cargo subcommand with `new`, `build`, and `validate`; `validate` checks entry points, ABI version, fingerprint, glibc symbol floor, and dynamic dependencies against the SLC library surface | owns: glibc floor and library surface lists | depends on: exasol-udf-sdk
- SLC image (Dockerfile): builds `exaudfclient` in `rust:1.98.1-trixie`, stages a shell-less runtime tree from `debian:trixie-slim`, and exports `lc-rs.tar.gz` from a `scratch` stage | owns: container tarball, bundled license files | depends on: exaudfclient
- install script (scripts/install.sh): builds the SLC, places it in BucketFS over HTTP or into an Exasol Personal deployment, and merges the `RUST` entry into `SCRIPT_LANGUAGES` with `ALTER SYSTEM` | owns: none | depends on: SLC image, exapump, BucketFS
- it (crates/it/): integration-test harness behind the `integration` feature; starts `exasol/docker-db`, uploads the SLC and UDF artifacts to BucketFS, registers `RUST`, and runs SQL | owns: test container lifecycle | depends on: exarrow-rs, testcontainers
- test-udfs (test-udfs/): fixture UDF cdylib crates loaded by runtime and integration tests | owns: none | depends on: exasol-udf-sdk, exasol-udf-macros
- exa-mock-db (crates/exa-mock-db/): dev-only mock engine; a ZMQ REP peer that drives a UDF client through handshake and run cycles with pre-encoded input and counts `MT_EMIT` frames | owns: mock session state | depends on: exa-proto, bench-schema
- benches (benches/): benchmark suite; `bench-schema` holds the shared column schema, `bench-udfs` the benchmark UDF cdylib, and `udf-bench` the Tier 2 `run` and `compare` driver against a live database | owns: benchmark result JSON | depends on: exasol-udf-sdk, exasol-udf-macros, it, exarrow-rs

## Data Flow

- DB -> exaudfclient -> exa-udf-runtime: the DB starts `exaudfclient <endpoint> lang=rust`; the runtime connects a REQ socket and sends `MT_CLIENT` with the endpoint URL as client name.
- DB -> exa-zmq-protocol -> exa-udf-runtime: `MT_INFO` and `MT_META` deliver script source and column metadata as `UdfMeta`; the runtime applies `%udf_debug_level` and opens a root tracing span tagged with pid, session, node, and VM id.
- script source -> artifact -> loader -> libudf.so: the runtime reads the `%udf_object <path>` directive, `dlopen`s the `.so`, resolves `__exa_udf_entry_<NAME>`, and rejects a wrong `abi_version` or `sdk_fingerprint`; a missing directive closes the session because JIT is not supported.
- UdfMeta -> loader/schema_check: the runtime checks the compiled output shape (EMITS or RETURNS) against `output_iter_type` and any annotated `input(...)`/`emits(...)` schema against the DB columns before any rows move.
- DB `MT_NEXT` -> rowset -> UDF `run`: each input batch becomes an input row set of SDK `Value` cells per group; the UDF reads it through `UdfContext`.
- UDF `ctx.emit` -> EmitBuffer -> `MT_EMIT` -> DB: emitted rows are checked against the output columns and written into the wire type blocks with the input row number; a flush sends one `MT_EMIT` when the byte estimate reaches 4,000,000 bytes, and a tail flush runs at the end of each group.
- UDF `emit_batch(RecordBatch)` -> Arrow IPC bytes -> EmitBuffer: the opt-in `emit-arrow` path crosses the `.so` boundary as Arrow IPC stream bytes and lands in the same emit buffer.
- DB `MT_RUN` -> single_call -> UDF hook -> `MT_RETURN`: in single-call mode each `MT_CALL` invokes one `SC_FN_*` hook (default output columns, virtual-schema adapter, import or export SQL generation) and returns JSON, or `MT_UNDEFINED_CALL` when the hook is absent.
- UDF `ctx.connection(name)` -> `MT_IMPORT` -> DB: CONNECTION-object credentials come back on demand as `ConnInfo` over the control channel.
- UDF `ctx.connect_back(&conn)` -> connect_back -> exarrow-rs -> Exasol SQL: the host opens a separate native-protocol session at the CONNECTION address; `query_for_each` converts one Arrow batch at a time to `Vec<Value>` rows and drops it before fetching the next.
- dispatch end -> cleanup hook -> `MT_FINISHED` -> exit(0): the optional `cleanup` hook runs once after dispatch; any dispatch or hook failure sends one error `MT_CLOSE` and the process exits non-zero.

## Interfaces

- Process invocation: `exaudfclient <endpoint> lang=rust [scriptOptionsParserVersion=N]`; fewer than 2 arguments exits 1 with `F-UDF-CL-RUST-0003`, a language other than `lang=rust` exits 2 with `F-UDF-CL-RUST-0002`, a runtime failure exits 1 with `F-UDF-CL-RUST-0001`.
- Container path: the binary lives at `/exaudf/exaudfclient` inside the SLC tree.
- Wire protocol: `localzmq+protobuf` per `crates/exa-proto/proto/zmqcontainer.proto` (vendored, provenance in `PROTO_SOURCES.md`); client REQ socket to DB REP socket, one prost-encoded payload frame per message, strict request/reply lockstep, pings answered inside the exchange.
- Protocol error close codes: `MT_CLOSE` with code 9001 for UDF and runtime errors and 1001 for an annotated-schema mismatch.
- Script directives: `%udf_object <path>` selects the `.so`; `%udf_debug_level <level>` sets the tracing level (default INFO).
- UDF ABI: one exported `extern "C" __exa_udf_entry_<NAME>() -> *const ExaUdfVTable`; the vtable carries `abi_version` (currently 11), a NUL-terminated `"SDK_VERSION:RUSTC_HASH"` fingerprint, `run`, optional `cleanup`, four optional single-call hooks, annotated schema JSON, and the output shape.
- UDF ABI calls: `run` and `cleanup` return 0 for ok, 1 for a user error with a `malloc`-allocated message in `error_out`, 2 for a panic; strings crossing the boundary are freed with `libc::free`.
- SDK API (crates.io `exasol-udf-sdk`): `UdfRun`, `UdfContext` (typed getters, `emit`, `next`, handshake metadata, `cluster_ip`, `connection`, `connect_back`), `ExaConnection` (`query`, `query_for_each`, `execute`, `execute_batch`, `begin`, `commit`, `rollback`), `Value`, `ExaType`, `ColumnInfo`; features `emit-arrow`, `import`, `export`, `test-support`.
- Proc macro (crates.io `exasol-udf-macros`): `#[exasol_udf]` with optional `input(...)` and `emits(...)` schema annotations.
- Spec-generation hook payload: `import_specification_rep` and `export_specification_rep` are passed to the hook as JSON that mirrors the proto field names, with every key always present.
- CLI (crates.io `cargo-exasol-udf`): `cargo exasol-udf new <path>`, `build [<path>] [--target <triple>]`, `validate <path> [--deny-unknown-deps]`.
- Install script: `scripts/install.sh` with `--host`, `--password`, `--bfs-password` for the BucketFS HTTP transport, or `--deployment <name>` for Exasol Personal; registers via `ALTER SYSTEM SET SCRIPT_LANGUAGES`, merging with existing entries.
- Container artifact: `lc-rs.tar.gz` holding the SLC root tree, uploaded to BucketFS under the SLC name (default `rustslc`).
- Benchmark driver: `udf-bench run --profile quick|full` writes one JSON result file; `udf-bench compare` compares two result sets.

## Constraints

- The Rust toolchain is pinned to 1.98.1 in `rust-toolchain.toml` and must equal the `rust:1.98.1-trixie` builder image in the `Dockerfile`, because the SDK fingerprint embeds the rustc version.
- The build is pure Cargo with no Bazel; `exa-proto` generates bindings at build time with a vendored `protoc`, so no system `protoc` is needed.
- `arrow` stays pinned to the version `exarrow-rs` uses (currently 58) so the host and the `.so` share one Arrow version.
- `main()` of `exaudfclient` ends with `std::process::exit`, because a normal return joins the connect-back Tokio threads, delays exit by about 10 seconds, and the DB watchdog then sends SIGABRT.
- The `.so` boundary carries only `repr(C)` types, C strings, SDK `Value` rows, and Arrow IPC bytes; protobuf and Arrow types never cross it.
- An `abi_version` or fingerprint mismatch at load produces an error instead of undefined behavior; `catch_unwind` in the generated shims turns a UDF panic into return code 2.
- `EMIT_BUFFER_LIMIT_BYTES` is 4,000,000 bytes, a flush target that matches the reference C++ SLC; a single row larger than the target is still sent as one `MT_EMIT`.
- Every emitted row carries the row number of its input row.
- The ZMQ transport polls with a 1 s receive/send timeout and retries `EAGAIN` up to a 120 s total wait per message.
- Connect-back runs on one process-wide `current_thread` Tokio runtime and never enters the ZMQ loop.
- Connect-back uses the native Exasol protocol, not WebSocket, so the DB-side session deregisters in under 1 s on disconnect.
- The SLC image has no shell, package manager, or coreutils; a UDF `.so` may link dynamically only against the staged library surface and the glibc floor listed in `crates/cargo-exasol-udf/`.
- The SLC supports x86_64 and aarch64 glibc targets.
- Only the precompiled `.so` path (Option A) is supported; a script without `%udf_object` closes with an unsupported error.

## External Dependencies

- Exasol DB (ZMQ REP peer): starts `exaudfclient` and drives the full protocol, sending input batches and receiving output | failure impact: no UDF can run
- exarrow-rs (crates.io): Arrow ADBC driver the host runtime uses to implement `ExaConnection`; UDFs never link it | failure impact: `ctx.connect_back()` returns an error and UDFs that use connect-back fail at runtime
- BucketFS: stores the SLC tarball and the precompiled UDF `.so` files | failure impact: the SLC cannot be registered and Option A UDFs cannot be loaded
- libzmq (via the `zmq` crate): ZeroMQ transport for the control channel | failure impact: the client cannot connect to the DB
- exapump CLI: runs the SQL that reads and writes `SCRIPT_LANGUAGES` in `scripts/install.sh` | failure impact: the install script cannot register the language
- Docker images `rust:1.98.1-trixie` and `debian:trixie-slim`: builder and runtime staging bases of the SLC image | failure impact: the SLC image cannot be built
- Docker image `exasol/docker-db`: live database for integration tests and the benchmark driver | failure impact: integration tests and Tier 2 benchmarks cannot run
- cargo-about and the GCC runtime-exception fetch (network): generate the third-party license bundle in the default build path | failure impact: the default SLC build in `scripts/install.sh` fails

## Exasol Data-Type Mapping

- The DB sends every column as one of 9 proto column types; `exa-zmq-protocol` maps each to the SDK `ExaType` in `column_from_pb`.
- `PB_DOUBLE` (`DOUBLE PRECISION`, `FLOAT`, `REAL`) -> `ExaType::Double` -> `Value::Double(f64)`.
- `PB_INT32` (`DECIMAL(p,0)` that fits `i32`) -> `ExaType::Int32` -> `Value::Int32(i32)`.
- `PB_INT64` (`DECIMAL(p,0)` that fits `i64`) -> `ExaType::Int64` -> `Value::Int64(i64)`.
- `PB_NUMERIC` (`DECIMAL(p,s)`, `BIGINT`, `NUMBER`) -> `ExaType::Numeric { precision, scale }` (defaults 18 and 0) -> `Value::Numeric(Decimal)`.
- `PB_DATE` (`DATE`) -> `ExaType::Date` -> `Value::Date(NaiveDate)`.
- `PB_TIMESTAMP` (`TIMESTAMP`) -> `ExaType::Timestamp { precision }`, with precision parsed from `type_name`, then `col.precision`, then default 3 -> `Value::Timestamp(NaiveDateTime)`.
- `PB_STRING` (`VARCHAR`, `CHAR`) -> `ExaType::Char { size }` when `type_name` starts with `CHAR`, otherwise `ExaType::String { size }` -> `Value::String`.
- `PB_BOOLEAN` (`BOOLEAN`) -> `ExaType::Boolean` -> `Value::Bool(bool)`.
- `PB_UNSUPPORTED` -> `ExaType::Unsupported`.
- `BIGINT` arrives as `PB_NUMERIC`, so `get_i64` accepts an integral `Value::Numeric` and errors only on a non-zero fractional part or an `i64` overflow.
- Only `PB_STRING` and `PB_TIMESTAMP` read `type_name`; all other proto types map directly.
- Refined string types keep a `String` wire payload; the `ExaType` variant, not the `Value` payload, carries the SQL distinction.
- Reference: <https://docs.exasol.com/db/latest/sql_references/data_types/datatypesoverview.htm>
