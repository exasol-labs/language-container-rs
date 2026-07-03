[language-container-rs](../README.md) › [docs](index.md) › Cargo Ecosystem

---

# Cargo Ecosystem

## Workspace layout

| Crate | Role | Published to crates.io? |
|-------|------|:-----------------------:|
| `exasol-udf-sdk` | Author-facing trait, types, macros | **yes** |
| `exasol-udf-macros` | Proc-macro crate for `#[exasol_udf]` | **yes** |
| `cargo-exasol-udf` | `cargo exasol-udf` build/validate subcommands | **yes** |
| `exa-udf-runtime` | ZMQ event loop; dispatches to `.so` via FFI | no (internal) |
| `exa-zmq-protocol` | ZMQ framing and message routing | no (internal) |
| `exa-proto` | Protobuf types (prost codegen) | no (internal) |
| `exaudfclient` | Thin ZMQ client used by the container entrypoint | no (internal) |
| `it` | Integration tests (Docker, Exasol 2026.latest) | no (internal) |

The three published crates are the customer-facing API; the rest are marked
`publish = false` and ship only as the prebuilt container binary.

`connect-back-query`, `connect-back-insert`, `connect-back-crunch`, `connect-back-cluster-ip`, and `spike-connect` are in `default-members` and build with plain `cargo build` under the unified 1.94 / edition-2024 toolchain.

`it` is excluded from `default-members` because it requires a live Exasol Docker container, not for any toolchain reason. Build and test it with:

```bash
cargo test -p it --features integration
```

The pinned workspace toolchain (`rust-toolchain.toml`) is 1.94.

## exasol-udf-sdk — author-facing crate

UDF authors add this to `[dependencies]`. It provides:

- **`UdfContext` trait** — `get`, `emit`, `next`, `cluster_ip`, `connection`, `connect_back`, plus the typed getters `get_i64` / `get_f64` / `get_str` / `get_bool` / `get_decimal` / `get_date` / `get_datetime` (each returns `Option<_>`, `None` for SQL NULL; `get_i64` also accepts a scale-0 `Numeric`)
- **`Value` enum** — `Int64`, `Float64`, `String`, `Bool`, `Null`, and the now strongly-typed `Numeric(Decimal)`, `Date(NaiveDate)`, `Timestamp(NaiveDateTime)`
- **`Decimal`** — `{ unscaled: i128, scale: u8 }` newtype (38-digit, no allocation) backing `Value::Numeric`
- **`ExaType`** — the canonical Exasol-type enum, now living in `exasol_udf_sdk::value` (re-exported by `exa-zmq-protocol`); covers the extended SQL types (`Char`, `TimestampWithLocalTimeZone`, intervals, `Geometry`, `Hashtype`, …)
- **`UdfError`** — typed error variants (`Type`, `User`, `Unimplemented`, …)
- **`ConnectionObject`** — credentials fetched from a named `CONNECTION` object

Feature flags:

| Feature | What it adds |
|---------|-------------|
| _(none)_ | Core trait + types; no async, no arrow |
| `connect-back` | `cluster_ip`, `connection`, `connect_back` on `UdfContext`; pulls in `arrow 58`, `exarrow-rs`, `tokio` |

## exa-udf-runtime — internal runtime

The ZMQ event loop that the container process runs. Responsibilities:

- Accepts incoming SLC connections on the ZMQ socket
- Parses `MT_RUN` / `MT_IMPORT_CONNECTION_INFORMATION` Protobuf messages
- `dlopen`s the UDF `.so` and calls the generated C entry point
- Catches panics across the FFI boundary and returns them as UDF errors

This crate is **internal** (`publish = false`) — UDF authors never depend on it.
It ships only as the prebuilt `exaudfclient` binary inside the container, built
from this workspace. Feature `connect-back` adds an embedded Tokio runtime and the
ADBC/exarrow-rs connect-back implementation.

## cargo-exasol-udf — build tool

Install from crates.io:

```bash
cargo install cargo-exasol-udf
```

Or from this workspace (development):

```bash
cargo install --path crates/cargo-exasol-udf
```

Subcommands:

| Subcommand | What it does |
|------------|-------------|
| `cargo exasol-udf new <path>` | Scaffold a new UDF crate with the correct `Cargo.toml` and `lib.rs` stub |
| `cargo exasol-udf build [<path>]` | Cross-compile to `x86_64-unknown-linux-musl` `.so` (release, symbols stripped) |
| `cargo exasol-udf validate <path>` | Inspect a compiled `.so`: enumerates all `__exa_udf_entry_*` symbols and validates each vtable |

`cargo exasol-udf build` sets `CARGO_TARGET_DIR`, selects the musl target, and passes `--release` — equivalent to `cargo build --target x86_64-unknown-linux-musl --release` but without needing to remember the flags.

### Multiple entry points per `.so`

A single `.so` may export any number of named entry points — one per `#[exasol_udf]`-annotated function. The macro derives the C symbol name from the Rust function identifier: `fn double_it` → `__exa_udf_entry_DOUBLE_IT`. The optional `name = "..."` attribute overrides the derived suffix verbatim.

`cargo exasol-udf validate` checks that at least one `__exa_udf_entry_*` symbol is present and that every vtable it finds is valid; it lists all found names.

> **Upgrade note (sdk < 0.14.0):** SDKs before 0.14.0 emitted a single bare `__exa_udf_entry` symbol. Rebuild any existing `.so` with sdk >= 0.14.0 so the runtime can locate the named symbol. The SQL `CREATE SCRIPT` name must match the UDF name (Rust fn ident, or the `name = "..."` override) — the runtime looks up `__exa_udf_entry_<UPPER_SNAKE_NAME>`.

## exa-zmq-protocol / exa-proto — protocol layer

`exa-proto` contains prost-generated Rust types for the SLC Protobuf schema. `exa-zmq-protocol` owns the ZMQ framing, message-type constants, and serialization helpers.

UDF authors never touch either crate. They are implementation details of `exa-udf-runtime`.

## it — integration tests

`crates/it` holds end-to-end tests that spin up `exasol/docker-db:2026.latest` via `testcontainers`, compile test UDFs, upload them, and run SQL assertions.

```bash
# Requires Docker
cargo test -p it --features integration
```

Tests run against `2026.latest` with `validateservercertificate=0`. Each test scenario maps to a UDF in `test-udfs/`.
