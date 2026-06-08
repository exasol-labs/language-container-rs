# Cargo Ecosystem

## Workspace layout

| Crate | Role | In default build? |
|-------|------|:-----------------:|
| `exasol-udf-sdk` | Author-facing trait, types, macros | yes |
| `exasol-udf-macros` | Proc-macro crate for `#[exasol_udf]` | yes |
| `exa-udf-runtime` | ZMQ event loop; dispatches to `.so` via FFI | yes |
| `cargo-exaudf` | `cargo exaudf` build/validate subcommands | yes |
| `exa-zmq-protocol` | ZMQ framing and message routing | yes |
| `exa-proto` | Protobuf types (prost codegen) | yes |
| `exaudfclient` | Thin ZMQ client used by the container entrypoint | yes |
| `it` | Integration tests (Docker, Exasol 2026.latest) | **no** |

`connect-back-query`, `connect-back-insert`, and `it` depend on `arrow 58` (edition 2024 transitive deps) and require Rust ≥ 1.85. Build them explicitly:

```bash
cargo +1.91 build -p connect-back-query
cargo +1.91 test  -p it --features integration
```

The pinned workspace toolchain (`rust-toolchain.toml`) is 1.84; these crates are excluded from `default-members` to keep `cargo build` fast and toolchain-clean.

## exasol-udf-sdk — author-facing crate

UDF authors add this to `[dependencies]`. It provides:

- **`UdfContext` trait** — `get`, `emit`, `next`, `cluster_ip`, `connection`, `connect_back`
- **`Value` enum** — `Int64`, `Float64`, `String`, `Bool`, `Numeric`, `Null`, and the rest
- **`UdfError`** — typed error variants (`Type`, `User`, `Unimplemented`, …)
- **`ConnectionObject`** — credentials fetched from a named `CONNECTION` object

Feature flags:

| Feature | What it adds |
|---------|-------------|
| _(none)_ | Core trait + types; no async, no arrow |
| `connect-back` | `cluster_ip`, `connection`, `connect_back` on `UdfContext`; pulls in `arrow 58`, `exarrow-rs`, `tokio` |

## exa-udf-runtime — operator-facing runtime

The ZMQ event loop that the container process runs. Responsibilities:

- Accepts incoming SLC connections on the ZMQ socket
- Parses `MT_RUN` / `MT_IMPORT_CONNECTION_INFORMATION` Protobuf messages
- `dlopen`s the UDF `.so` and calls the generated C entry point
- Catches panics across the FFI boundary and returns them as UDF errors

UDF authors do **not** depend on `exa-udf-runtime` directly. Operators who build a custom container binary embed it:

```toml
[dependencies]
exa-udf-runtime = { version = "0.3" }
```

Feature `connect-back` adds an embedded Tokio runtime and the ADBC/exarrow-rs connect-back implementation.

## cargo-exaudf — build tool

Install from this workspace:

```bash
cargo install --path crates/cargo-exaudf
```

Subcommands:

| Subcommand | What it does |
|------------|-------------|
| `cargo exaudf new <path>` | Scaffold a new UDF crate with the correct `Cargo.toml` and `lib.rs` stub |
| `cargo exaudf build [<path>]` | Cross-compile to `x86_64-unknown-linux-musl` `.so` (release, symbols stripped) |
| `cargo exaudf validate <path>` | Inspect a compiled `.so` for the expected exported symbol |

`cargo exaudf build` sets `CARGO_TARGET_DIR`, selects the musl target, and passes `--release` — equivalent to `cargo build --target x86_64-unknown-linux-musl --release` but without needing to remember the flags.

## exa-zmq-protocol / exa-proto — protocol layer

`exa-proto` contains prost-generated Rust types for the SLC Protobuf schema. `exa-zmq-protocol` owns the ZMQ framing, message-type constants, and serialization helpers.

UDF authors never touch either crate. They are implementation details of `exa-udf-runtime`.

## it — integration tests

`crates/it` holds end-to-end tests that spin up `exasol/docker-db:2026.latest` via `testcontainers`, compile test UDFs, upload them, and run SQL assertions.

```bash
# Requires Rust >= 1.85 and Docker
cargo +1.91 test -p it --features integration
```

Tests run against `2026.latest` with `validateservercertificate=0`. Each test scenario maps to a UDF in `test-udfs/`.
