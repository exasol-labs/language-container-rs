# lc-rs — Technical Design: Rust-native Exasol Language Container

## Context

Exasol Language Containers (SLCs) extend the database with additional language runtimes. Currently only Python, Java, and R are supported. This document describes the design for a production-quality Rust SLC that:

- Allows users to write Exasol UDFs in Rust
- Implements the identical `localzmq+protobuf` wire protocol used by existing SLCs
- Offers all UDF capabilities: scalar, set/EMITS, import spec generation, export spec generation
- Uses `exarrow-rs` (crates.io, v0.12.7) for connect-back to Exasol from within UDFs

The existing C++ launcher (`script-languages-release/exaudfclient/exaudfclient.cc`) loads `libexaudflib_complete.so` via `dlmopen` to avoid symbol conflicts with the DB's protobuf. The Rust replacement implements the protocol directly in-process — no `libexaudflib` needed.

---

## 1. Cargo Workspace Structure

```
lc-rs/
  Cargo.toml                        # [workspace], members = all crates below
  rust-toolchain.toml               # pinned channel (e.g. stable 1.92) — MUST match container
  Cargo.lock
  crates/
    exa-proto/                      # prost-generated protobuf bindings
      build.rs                      # prost-build over vendored proto
      proto/zmqcontainer.proto      # vendored from exasol/script-languages (see §2)
      src/lib.rs
    exa-zmq-protocol/               # ZMQ DEALER transport + typed state machine
      src/lib.rs
      src/transport.rs
      src/messages.rs
      src/loop_.rs
      src/meta.rs                   # ColumnMeta, UdfMeta, IterType
      src/error.rs
    exasol-udf-sdk/                 # PUBLIC: what UDF authors depend on
      src/lib.rs
      src/context.rs                # UdfContext + UdfRun traits
      src/value.rs                  # Value enum, ExaType
      src/meta.rs                   # UdfMeta, ColumnSpec
      src/connect_back.rs           # exarrow-rs bridge (feature: connect-back)
      src/arrow.rs                  # RecordBatch <-> column-set conversion
      src/error.rs
      src/abi.rs                    # stable C-ABI contract (ExaUdfVTable)
    exasol-udf-macros/              # proc-macro: #[exasol_udf]
      src/lib.rs
    exa-udf-runtime/                # host: loads .so, drives protocol <-> SDK
      src/lib.rs
      src/loader.rs                 # libloading + ABI/fingerprint gating
      src/dispatch.rs               # MT_RUN/MT_NEXT/MT_EMIT <-> UdfContext bridge
      src/single_call.rs            # SC_FN_* handling
      src/compiler.rs               # JIT: write source to /tmp, cargo build --offline
      src/rowset.rs                 # Arrow builders for input buffering
    exaudfclient/                   # the binary: ships as /exaudf/exaudfclient
      src/main.rs
    cargo-exaudf/                   # CLI: scaffold + build + validate UDF .so locally
      src/main.rs
```

### Dependency graph (acyclic)

```
exaudfclient --> exa-udf-runtime --> exa-zmq-protocol --> exa-proto
                      |
                      +--> exasol-udf-sdk  (ABI types)
exasol-udf-sdk --> exasol-udf-macros (re-export), arrow, exarrow-rs (opt feature)
```

### Key crate dependencies

| Concern | Crate | Notes |
|---|---|---|
| ZMQ | `zmq = "0.10"` (libzmq C bindings) | Matches libzmq 4.3.5 in DB; sync DEALER keeps hot path simple |
| Protobuf | `prost = "0.13"`, `prost-build` in build.rs | No runtime protoc |
| Dynamic load | `libloading = "0.8"` | Load user `libudf.so` |
| Arrow | `arrow = "58"` | Pinned to exarrow-rs version for zero-copy RecordBatch reuse |
| DB connect-back | crates.io dependency `exarrow-rs` v0.12.7 | Behind `connect-back` Cargo feature flag |
| Async (connect-back only) | `tokio = "1"` | Dedicated current_thread runtime, never enters ZMQ loop |
| Proc macro | `syn = "2"`, `quote = "1"`, `proc-macro2` | |
| Errors | `thiserror`, `anyhow` (binary only) | |
| Logging | `tracing`, `tracing-subscriber` | stderr only; Exasol captures stderr to UDF log |

`exarrow-rs` is a crates.io dependency; `arrow = "58"` is pinned once in `[workspace.dependencies]` and shared by `exasol-udf-sdk` and `exarrow-rs` to avoid a duplicate Arrow copy.

---

## 2. Protocol Layer

### 2.1 Obtaining zmqcontainer.proto

Vendor into `crates/exa-proto/proto/zmqcontainer.proto` from:

```
https://github.com/exasol/script-languages/raw/master/exaudfclient/base/exaudflib/zmqcontainer.proto
```

Locally available once initialized:

```
script-languages-release/exaudfclient/base/exaudflib/zmqcontainer.proto
# initialize with: git submodule update --init  (in script-languages-release)
```

Pin to a specific git commit in a `PROTO_SOURCES.md`. The proto uses `syntax = "proto2"` with `optimize_for = LITE_RUNTIME` — prost ignores the optimize_for directive, so no special `prost_build::Config` is needed.

### 2.2 Key proto message types

**Enums:**
- `message_type`: `MT_CLIENT`, `MT_INFO`, `MT_META`, `MT_RUN`, `MT_NEXT`, `MT_EMIT`, `MT_DONE`, `MT_CLEANUP`, `MT_FINISHED`, `MT_CLOSE`, `MT_CALL`, `MT_RETURN`, `MT_UNDEFINED_CALL`, `MT_PING_PONG`, `MT_TRY_AGAIN`, `MT_RESET`
- `column_type`: `PB_DOUBLE`, `PB_INT32`, `PB_INT64`, `PB_NUMERIC`, `PB_TIMESTAMP`, `PB_DATE`, `PB_STRING`, `PB_BOOLEAN`
- `iter_type`: `PB_EXACTLY_ONCE` (scalar), `PB_MULTIPLE` (set/EMITS)
- `single_call_function_id`: `SC_FN_DEFAULT_OUTPUT_COLUMNS`, `SC_FN_GENERATE_SQL_FOR_IMPORT_SPEC`, `SC_FN_GENERATE_SQL_FOR_EXPORT_SPEC`, `SC_FN_VIRTUAL_SCHEMA_ADAPTER_CALL`

**Envelopes:**
- `exascript_request` — sent by the client (us) to the DB
- `exascript_response` — sent by the DB to the client

**Payload messages:**
- `exascript_metadata` — column definitions, iter_type, input/output column counts
- `exascript_table_data` — row batches (column-oriented)
- `exascript_client`, `exascript_info` — handshake (script source text, session params, `single_call_function_id`, `connection_id`)
- `exascript_next_data_rep` — input batch from DB
- `exascript_emit_data_req` — output batch to DB
- `connection_information_rep` — credentials for connect-back (IMPORT/EXPORT path)
- `exascript_single_call_rep`, `import_specification_rep`, `export_specification_rep` — SC_FN results

### 2.3 ZMQ transport and message loop (`exa-zmq-protocol`)

The DB acts as ROUTER; the client opens a **DEALER** to `ipc://<socket_path>`. Each protobuf message is one ZMQ frame (single-part).

```rust
// transport.rs
pub struct ZmqTransport { socket: zmq::Socket /* DEALER */ }

impl ZmqTransport {
    pub fn connect(ipc_path: &str) -> Result<Self, ProtocolError>;
    pub fn send(&self, req: &ExascriptRequest) -> Result<(), ProtocolError>;
    pub fn recv(&self) -> Result<ExascriptResponse, ProtocolError>;  // blocking poll
}

// loop_.rs — pure state machine, no I/O, fully unit-testable
pub enum HostEvent<'a> {
    Info(&'a InfoPayload),
    Meta(&'a MetadataPayload),
    InputRows(RowBatchView<'a>),
    SingleCall { fn_id: SingleCallFn, args: &'a [Value] },
    Cleanup, Finished, Close, PingPong, TryAgain, Reset,
}

pub enum HostAction {
    SendNext, Emit(OutBatch), Return(ReturnValue), UndefinedCall, Done, Cleanup, Close,
}

pub struct Protocol { phase: Phase, connection_id: u64, iter_type: IterType }

impl Protocol {
    pub fn on_response(&mut self, resp: &ExascriptResponse) -> Result<HostEvent<'_>, ProtocolError>;
    pub fn next_request(&mut self, action: HostAction) -> ExascriptRequest;
}
```

**Handshake and dispatch sequence:**

1. Send `MT_CLIENT` → recv `MT_INFO` (script source, session, `single_call_function_id`, `connection_id`)
2. Send `MT_META` (request metadata) → recv `MT_META` (column definitions, `iter_type`)
3. Compile/load UDF (see §3)
4. If scalar/set: send `MT_RUN` → exchange `MT_NEXT` / `MT_EMIT` in a loop → send `MT_DONE` → handle `MT_CLEANUP` / `MT_FINISHED` / `MT_CLOSE`
5. If single-call: recv `MT_CALL` → send `MT_RETURN` or `MT_UNDEFINED_CALL`
6. Side cases: `MT_PING_PONG` (echo immediately), `MT_TRY_AGAIN` (backoff then re-poll), `MT_RESET` (restart input iteration)

Error close path: on UDF failure, serialize the error string and send it through the standard close sequence so it surfaces in the Exasol UDF error message. Error code prefix: `F-UDF-CL-RUST-####`.

---

## 3. UDF Execution Model

### Options considered

| | A: precompiled `.so` from BucketFS | B: static image | C: JIT-compile in container |
|---|---|---|---|
| Author workflow | `cargo exaudf build`, upload `.so` | Rebuild whole image | Upload Rust source, SLC compiles on first call |
| Toolchain in container | none | none | full Rust toolchain (~1.4 GB) |
| Mirrors existing SLC model | closest to Java | no | yes (Java also JIT-compiles) |
| ABI risk | fingerprint-checked at dlopen | none | none (same container toolchain) |

**Recommendation: C as primary (JIT), A as the fast path. Both share the same loader.**

- **Option C (JIT):** `exascript_info.source_code` contains the user's Rust source. The runtime writes it to `/tmp/udf-<sha256>/src/lib.rs` inside a pre-baked template crate (`/opt/lc-rs/template/` — includes `Cargo.toml` with `exasol-udf-sdk` dependency and `.cargo/config.toml` pointing to the offline vendored registry at `/opt/lc-rs/vendor/`). Runs `cargo build --release --offline`. Caches the resulting `libudf.so` at `/tmp/udf-cache/<hash>/` keyed by `sha256(source ++ SDK_VERSION ++ RUSTC_VERSION)`. A warm-cache hit skips compilation entirely.

- **Option A (precompiled):** A `%`-option in the script source (e.g. `%udf_object /buckets/bfsdefault/default/path/libudf.so`) names a prebuilt `.so` in BucketFS. The runtime reads the path and skips compilation. Authors use `cargo exaudf build` locally (or in CI) against the exact same toolchain pinned in `rust-toolchain.toml`.

### Plugin ABI (`exasol-udf-sdk::abi`)

Both options produce a `crate-type = ["cdylib"]` exporting one C-ABI symbol. The `extern "C"` boundary is the only ABI crossing; the rich trait objects stay entirely within the host (compiled with the same toolchain for Option C, fingerprint-gated for Option A).

```rust
pub const EXA_UDF_ABI_VERSION: u32 = 1;

#[repr(C)]
pub struct ExaUdfVTable {
    pub abi_version:              u32,
    pub sdk_fingerprint:          *const u8,  // "SDK_VERSION:RUSTC_HASH\0"
    pub create:                   extern "C" fn() -> *mut c_void,
    pub destroy:                  extern "C" fn(*mut c_void),
    pub run:                      extern "C" fn(udf: *mut c_void, ctx: *mut c_void) -> i32,
    pub default_output_columns:   Option<extern "C" fn(*mut c_void, *mut c_void) -> i32>,
    pub generate_sql_import:      Option<extern "C" fn(*mut c_void, *mut c_void) -> i32>,
    pub generate_sql_export:      Option<extern "C" fn(*mut c_void, *mut c_void) -> i32>,
}

// Single well-known exported symbol:
#[no_mangle]
pub extern "C" fn __exa_udf_entry() -> *const ExaUdfVTable { &VT }
```

`loader.rs` in `exa-udf-runtime`:
1. `libloading::Library::new(so_path)` — dlopen
2. Resolve `__exa_udf_entry`
3. Check `vtable.abi_version == EXA_UDF_ABI_VERSION` — hard error on mismatch
4. Check `vtable.sdk_fingerprint` matches host's fingerprint — hard error on mismatch (guards Option A)
5. Call `vtable.create()` to allocate the UDF struct

Option-C `.so` files are fingerprint-guaranteed (same container toolchain). For Option A, the fingerprint check at dlopen gives a clear error rather than UB.

---

## 4. UDF Context API (`exasol-udf-sdk`)

### Value model

```rust
// value.rs
pub enum Value {
    Null,
    Int32(i32),
    Int64(i64),
    Double(f64),
    Numeric(i128, u8),          // (unscaled_value, scale), max precision 36 — Decimal128 semantics
    Bool(bool),
    String(String),
    Date(chrono::NaiveDate),
    Timestamp(chrono::NaiveDateTime),  // no timezone (Exasol TIMESTAMP)
}

pub enum ExaType {
    Int32, Int64, Double,
    Numeric { precision: u8, scale: u8 },
    Bool,
    String { size: u32 },
    Date, Timestamp,
}
```

### Traits

```rust
// context.rs

pub trait UdfRun {
    fn run(&mut self, ctx: &mut dyn UdfContext) -> Result<(), UdfError>;

    // Optional single-call hooks; default = Err(UdfError::Unimplemented) -> MT_UNDEFINED_CALL
    fn default_output_columns(&mut self, _ctx: &dyn MetaContext) -> Result<Vec<ColumnSpec>, UdfError>
        { Err(UdfError::Unimplemented) }
    fn generate_sql_for_import_spec(&mut self, _spec: &ImportSpec) -> Result<String, UdfError>
        { Err(UdfError::Unimplemented) }
    fn generate_sql_for_export_spec(&mut self, _spec: &ExportSpec) -> Result<String, UdfError>
        { Err(UdfError::Unimplemented) }
}

pub trait UdfContext {
    // Row iteration
    fn next(&mut self) -> Result<bool, UdfError>;
    fn reset(&mut self) -> Result<(), UdfError>;
    fn emit(&mut self, values: &[Value]) -> Result<(), UdfError>;

    // Column introspection
    fn column_count(&self) -> usize;
    fn column_name(&self, index: usize) -> &str;
    fn column_type(&self, index: usize) -> ExaType;
    fn column_index(&self, name: &str) -> Option<usize>;

    // Typed accessors (None = SQL NULL)
    fn get_value(&self, index: usize) -> Option<&Value>;
    fn get_i64(&self, index: usize)       -> Result<Option<i64>, UdfError>;
    fn get_f64(&self, index: usize)       -> Result<Option<f64>, UdfError>;
    fn get_string(&self, index: usize)    -> Result<Option<&str>, UdfError>;
    fn get_bool(&self, index: usize)      -> Result<Option<bool>, UdfError>;
    fn get_decimal(&self, index: usize)   -> Result<Option<(i128, u8)>, UdfError>;
    fn get_date(&self, index: usize)      -> Result<Option<chrono::NaiveDate>, UdfError>;
    fn get_timestamp(&self, index: usize) -> Result<Option<chrono::NaiveDateTime>, UdfError>;

    // Arrow batch fast path — zero-copy from internal Arrow builders
    fn get_arrow_batch(&mut self, max_rows: usize) -> Result<arrow::array::RecordBatch, UdfError>;
    fn emit_arrow_batch(&mut self, batch: &arrow::array::RecordBatch) -> Result<(), UdfError>;

    // Session metadata (script name, schema, DB version, options, scope variables)
    fn meta(&self) -> &UdfMeta;

    // Connect back to Exasol via exarrow-rs
    #[cfg(feature = "connect-back")]
    fn connect_back(&self) -> Result<exarrow_rs::adbc::Connection, UdfError>;
    #[cfg(feature = "connect-back")]
    fn connect_back_with(&self, opts: ConnectBackOptions) -> Result<exarrow_rs::adbc::Connection, UdfError>;
}
```

### Arrow column mapping

| Exasol/proto type | Arrow type |
|---|---|
| PB_INT32 | Int32Array |
| PB_INT64 | Int64Array |
| PB_DOUBLE | Float64Array |
| PB_NUMERIC | Decimal128Array |
| PB_BOOLEAN | BooleanArray |
| PB_STRING | StringArray |
| PB_DATE | Date32Array |
| PB_TIMESTAMP | TimestampMicrosecondArray (no TZ) |

`exascript_table_data` in the proto is column-oriented, which maps directly to Arrow arrays. `get_arrow_batch` materializes them in-place without row-by-row allocation. `emit_arrow_batch` disassembles a `RecordBatch` into `exascript_emit_data_req` column blocks. Because `arrow = "58"` is pinned across both crates, batches returned by `ctx.connect_back()?.query(sql).await?` can be passed directly to `ctx.emit_arrow_batch` — no conversion.

### connect-back (`connect_back.rs`)

exarrow-rs is fully async (tokio). The ZMQ dispatch loop is synchronous (blocking). The SDK owns a dedicated `OnceLock<tokio::runtime::Runtime>` (current_thread, single-thread) used exclusively for connect-back calls:

```rust
static CONNECT_BACK_RT: OnceLock<Runtime> = OnceLock::new();

fn connect_back_rt() -> &'static Runtime {
    CONNECT_BACK_RT.get_or_init(|| {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("connect-back runtime")
    })
}

pub fn connect_back(meta: &UdfMeta) -> Result<Connection, UdfError> {
    connect_back_rt().block_on(
        Connection::from_params(params_from_meta(meta))
    ).map_err(UdfError::ConnectBack)
}
```

exarrow-rs also provides `blocking_import_*` / `blocking_export_*` wrappers that internally use their own `OnceLock<Runtime>`, so those can be called directly from synchronous UDF code without additional wrapping.

**Credentials** for connect-back (in preference order):
1. DB-provided `connection_information_rep` via `PB_IMPORT_CONNECTION_INFORMATION` — the standard IMPORT/EXPORT path; carries host + user + password
2. Named `CONNECTION` object: `%connection MY_CONN` in script source, resolved by the DB and delivered via `MT_IMPORT`
3. Explicit credentials in script options (last resort; document as not recommended)

---

## 5. Proc Macro (`exasol-udf-macros`)

`#[exasol_udf]` is an attribute macro applied to a struct that implements `UdfRun`:

```rust
use exasol_udf_sdk::prelude::*;

#[exasol_udf]
pub struct DoubleIt;

impl UdfRun for DoubleIt {
    fn run(&mut self, ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
        while ctx.next()? {
            let x = ctx.get_i64(0)?.unwrap_or(0);
            ctx.emit(&[Value::Int64(x * 2)])?;
        }
        Ok(())
    }
}
```

What the macro generates:

1. `extern "C"` vtable shims: `create`, `destroy`, `run`, plus optional shims for `default_output_columns`, `generate_sql_import`, `generate_sql_export` (shims for unimplemented methods return `UdfError::Unimplemented` which the host maps to `MT_UNDEFINED_CALL`)
2. `static VT: ExaUdfVTable` with:
   - `abi_version = EXA_UDF_ABI_VERSION`
   - `sdk_fingerprint` from `env!("EXA_SDK_FINGERPRINT")` — a build.rs-baked string of the form `"SDK_VERSION:RUSTC_HASH\0"`
3. `#[no_mangle] pub extern "C" fn __exa_udf_entry() -> *const ExaUdfVTable { &VT }`
4. Linker-level duplicate guard: two `#[exasol_udf]` in one crate produce a duplicate `__exa_udf_entry` symbol → link error
5. `std::panic::catch_unwind` wrapper in the `run` shim — converts Rust panics to `i32 = 2` error code, sets an error message on the context bridge

Optional typed schema annotation:

```rust
#[exasol_udf(input(x: i64, label: String), emits(result: i64))]
```

Generates static schema validation against `exascript_metadata` at UDF load time, producing a clear error if the DB-declared types don't match.

---

## 6. Container Runtime Binary (`exaudfclient/src/main.rs`)

Replicates the C++ launcher contract exactly (verified from `exaudfclient.cc`):

```
INVOCATION: /exaudf/exaudfclient <ipc_socket_path> lang=rust [scriptOptionsParserVersion=1|2]
```

```rust
fn main() -> ExitCode {
    init_tracing_to_stderr();
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 || args.len() > 4 {
        eprintln!("Usage: {} <socket> lang=rust [scriptOptionsParserVersion=1|2]", args[0]);
        return ExitCode::from(1);
    }
    if args[2] != "lang=rust" {
        eprintln!("F-UDF-CL-RUST-0001: lang '{}' not supported", args[2]);
        return ExitCode::from(2);
    }
    let socket_path = &args[1];
    // env var takes priority over CLI arg, matching C++ behavior
    let parser_version = resolve_parser_version(&args, std::env::var("SCRIPT_OPTIONS_PARSER_VERSION").ok());
    std::env::set_var("HOME", "/tmp");   // matches C++ launcher

    match exa_udf_runtime::Runtime::new(socket_path, parser_version).and_then(|mut r| r.run()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => { eprintln!("F-UDF-CL-RUST-0001: {e}"); ExitCode::FAILURE }
    }
}
```

`exa_udf_runtime::Runtime::run()` orchestrates:

1. Handshake: send `MT_CLIENT`, recv `MT_INFO` → extract script source, `single_call_function_id`, connection info
2. Send `MT_META`, recv `MT_META` → build `Vec<ColumnMeta>` and `iter_type`
3. Resolve artifact: parse `%udf_object` option → load `.so` (Option A), else JIT-compile source to `.so` (Option C)
4. Load: `loader.rs` dlopen, check ABI version + fingerprint, call `vtable.create()`
5. Build `HostContextBridge` (implements `UdfContext`, wraps transport + Arrow column builders + emit buffer)
6. Dispatch:
   - **Single-call** (`single_call_function_id != SC_FN_NIL`): route `MT_CALL` to the appropriate vtable shim; reply `MT_RETURN` or `MT_UNDEFINED_CALL`
   - **Scalar/set**: send `MT_RUN`, call `vtable.run(udf, &mut bridge)`. Bridge's `next()` issues `MT_NEXT` and populates Arrow builders from `exascript_next_data_rep`; `emit()` accumulates and flushes `MT_EMIT` in batches; on `run` return send `MT_DONE`
   - Handle `MT_PING_PONG`, `MT_TRY_AGAIN`, `MT_RESET`, `MT_CLEANUP`, `MT_FINISHED`, `MT_CLOSE`
7. On error: serialize error message into close path, call `vtable.destroy(udf)`, drop `Library`, return failure

`Library` is held alive for the session lifetime and dropped after `destroy()`.

---

## 7. Docker Container Design

### Two profiles

**`slim`** — Option A (precompiled `.so`) only. No Rust toolchain. Image size ~150 MB.

**`jit`** — Option C (JIT compile) + Option A. Includes Rust toolchain + vendored Cargo registry. Image size ~1.4 GB.

### Multi-stage Dockerfile (conceptual — in the exaslct flavor this is split across layer Dockerfiles)

```dockerfile
# ---- Stage: builder ----
FROM rust:1.92-bookworm AS builder
RUN apt-get update && apt-get install -y libzmq3-dev protobuf-compiler pkg-config
WORKDIR /src
COPY . .
RUN cargo build --release -p exaudfclient
# Vendor SDK deps for in-container JIT builds
RUN mkdir /vendor && cargo vendor \
      --manifest-path crates/exasol-udf-sdk/Cargo.toml /vendor

# ---- Stage: runtime ----
FROM debian:12-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
      libzmq5 ca-certificates locales && \
    sed -i 's/# en_US.UTF-8/en_US.UTF-8/' /etc/locale.gen && locale-gen
ENV LANG=en_US.UTF-8

# JIT profile only
COPY --from=rust:1.92-bookworm /usr/local/cargo  /usr/local/cargo
COPY --from=rust:1.92-bookworm /usr/local/rustup /usr/local/rustup
ENV PATH=/usr/local/cargo/bin:$PATH \
    CARGO_HOME=/usr/local/cargo \
    RUSTUP_HOME=/usr/local/rustup

RUN mkdir -p /exaudf /build_info /conf /buckets \
             /opt/lc-rs/vendor /opt/lc-rs/template
COPY --from=builder /src/target/release/exaudfclient /exaudf/exaudfclient
COPY --from=builder /vendor /opt/lc-rs/vendor
COPY container/template-crate/    /opt/lc-rs/template/
COPY container/cargo-offline.toml /opt/lc-rs/template/.cargo/config.toml
COPY build_info/language_definitions.json /build_info/language_definitions.json
```

### `/build_info/language_definitions.json`

```json
{
  "schema_version": 2,
  "language_definitions": [{
    "protocol": "localzmq+protobuf",
    "aliases": ["RUST"],
    "parameters": [{"key": "lang", "value": "rust"}],
    "udf_client_path": {"executable": "/exaudf/exaudfclient"},
    "deprecation": null
  }]
}
```

### `language_definition` template (for `ALTER SESSION`)

```
RUST=localzmq+protobuf:///{{ bucketfs_name }}/{{ bucket_name }}/{{ path_in_bucket }}{{ release_name }}?lang=rust#buckets/{{ bucketfs_name }}/{{ bucket_name }}/{{ path_in_bucket }}{{ release_name }}/exaudf/exaudfclient
```

`rust-toolchain.toml` must pin the exact toolchain version baked into the container. The `EXA_SDK_FINGERPRINT` environment variable (set in the build) embeds this version into the `sdk_fingerprint` field of every compiled vtable.

---

## 8. Build Pipeline Integration (exaslct flavor)

Create `flavors/standard-EXASOL-all-rust/` in `script-languages-release` (fork or new branch), mirroring the python-3.12 flavor's `flavor_base` layer DAG:

```
flavors/standard-EXASOL-all-rust/
  packages.yml
  ci.json
  FLAVOR_DESCRIPTION.md
  flavor_base/
    derived_from
    packages.yml                    # build-time: libzmq3-dev, protobuf-compiler, pkg-config
    language_definition             # RUST=... template string
    language_definitions.json
    build_steps.py                  # exaslct task graph (subclass DockerFlavorAnalyzeImageTask)
    udfclient_deps/Dockerfile       # base Linux + build toolchain
    language_deps/Dockerfile        # Rust toolchain (jit) or no-op (slim)
    build_deps/Dockerfile           # cargo, libzmq3-dev, protobuf-compiler
    build_run/Dockerfile            # cargo build; cp exaudfclient; cargo vendor
    flavor_base_deps/Dockerfile     # runtime: libzmq5, ca-certificates, locales
    release/Dockerfile
    security_scan/Dockerfile
```

`build_run/Dockerfile` key steps:

```dockerfile
FROM {{language_deps}}
RUN mkdir /exaudfclient /exaudf /opt/lc-rs/vendor /opt/lc-rs/template
COPY /lc-rs /exaudfclient
WORKDIR /exaudfclient
RUN cargo build --release -p exaudfclient && \
    cp target/release/exaudfclient /exaudf/exaudfclient && \
    cargo vendor crates/exasol-udf-sdk /opt/lc-rs/vendor
COPY container/template-crate/    /opt/lc-rs/template/
COPY container/cargo-offline.toml /opt/lc-rs/template/.cargo/config.toml
RUN rm -rf target ~/.cargo/registry/cache
```

`build_steps.py` adds `lc-rs` to `get_additional_build_directories_mapping`:

```python
def get_additional_build_directories_mapping(self):
    return {"lc-rs": "lc-rs"}   # analogous to {"exaudfclient": "exaudfclient"} in python flavor
```

No Bazel — pure Cargo. The build system's only requirement is that `/exaudf/exaudfclient` exists in the release image.

---

## 9. Testing Strategy

1. **Unit — protocol state machine** (`exa-zmq-protocol`): feed decoded `ExascriptResponse` fixtures, assert emitted `ExascriptRequest`s and `HostEvent` variants. Round-trip prost encode/decode for every message type. Golden-byte fixtures captured from the emulator or a real DB session to lock wire compatibility.

2. **Unit — SDK context** (`exa-udf-runtime`): `HostContextBridge` against an in-memory fake transport. Verify typed accessors, NULL handling, Decimal/Date parsing, Arrow batch conversion (all PB column types → arrays and back), emit arity checks, MT_RESET.

3. **Unit — loader/ABI** (`exa-udf-runtime::loader`): build a fixture UDF crate in `#[test]`, dlopen it, assert ABI/fingerprint gating — including a deliberately mismatched `abi_version` and a mismatched `sdk_fingerprint`, both must produce a clear error rather than UB.

4. **Unit — macro** (`exasol-udf-macros`): `trybuild` tests — valid `#[exasol_udf]` compiles; two `#[exasol_udf]` in one crate fails at link time; typed annotation with wrong column count fails at load time.

5. **Integration — emulator**: `script-languages-release/emulator/` (from the initialized submodule) runs a UDF process without a full DB. Wire the Rust `exaudfclient` to it. Cover: scalar, set/EMITS, MT_RESET, all three SC_FN_* single-call functions, error propagation, connect-back (mock DB response).

6. **End-to-end (real DB)**: use `exasol/slc_release` Python tooling + Exasol docker-db:
   - Write a Rust UDF, `cargo exaudf build` → upload `.so` to BucketFS → `ALTER SESSION SET SCRIPT_LANGUAGES='RUST=...'` → `CREATE RUST SCALAR SCRIPT` → run → assert result
   - JIT path: paste Rust source directly into `CREATE SCRIPT` body, observe cold-compile delay then correct results on re-run
   - connect-back: a UDF that calls `ctx.connect_back()?.execute("SELECT 42").await` and emits the result

7. **Toolchain-match test**: assert container's `rustc --version` equals `rust-toolchain.toml`; build an Option-A `.so` in CI with that exact toolchain and verify it loads in the container; build one with a different version and verify the fingerprint check rejects it.

---

## 10. Known Gaps & Risks

| # | Risk | Mitigation |
|---|---|---|
| 1 | `zmqcontainer.proto` not on disk (uninitialized submodule) | First step: `git submodule update --init` in `script-languages-release`; or fetch from GitHub raw URL and vendor |
| 2 | Exact ZMQ loop sequence (MT_RESET, error close path, MT_TRY_AGAIN timing) not fully verified | Validate against live emulator before locking; proto enums + launcher code are the ground truth |
| 3 | connect-back credentials for non-IMPORT UDFs | Test empirically with real DB; named `CONNECTION` object is safest fallback; `PB_IMPORT_CONNECTION_INFORMATION` only guaranteed for IMPORT scripts |
| 4 | Async/sync duality: nested tokio runtimes | SDK uses a dedicated `OnceLock<Runtime>`; document that user UDFs must not spawn their own runtime; add a `thread_local` guard that panics on re-entry |
| 5 | JIT image size (~1.4 GB) | On-disk compile cache at `/tmp/udf-cache/`; offer `slim` profile for latency-sensitive deployments; `sccache` inside container is an option |
| 6 | Rust ABI instability across compiler versions | For Option C: same container toolchain guarantees match. For Option A: `sdk_fingerprint` check at dlopen — mismatch → clear error, not UB |
| 7 | Panic safety: UDF runs in-process | `catch_unwind` in `run` shim; template crate sets `[profile.release] panic = "abort"`; document that panics surface as UDF errors |
| 8 | Arrow version coupling | `arrow = "58"` pinned via `[workspace.dependencies]`; bump must be coordinated between `exasol-udf-sdk` and `exarrow-rs` |

---

## 11. UDF Authoring Model (Option A — Precompiled `.so`)

This section captures design decisions made after the initial design doc, during the mission iteration session.

### 11.1 Author workflow end-to-end

```
1. cargo exaudf new my-udf              # scaffold crate with correct Cargo.toml
2. implement UdfRun for MyStruct        # write business logic
3. cargo exaudf build                   # produces fully-static musl .so
4. exapump udf deploy ...               # upload + CREATE SCRIPT
5. SELECT my_schema.my_udf(x) FROM t   # call it
```

### 11.2 musl target

`cargo exaudf build` always compiles to `x86_64-unknown-linux-musl`. The target triple is hidden from the author. On first run, `cargo exaudf` calls `rustup target add x86_64-unknown-linux-musl` if the target is absent.

This makes the `.so` fully self-contained: all Rust dependencies are statically linked, no glibc dependency. Authors may add any `crates.io` dependency to their `Cargo.toml`; it is statically linked into the artifact automatically.

System-level C libraries are out of scope for the slim container. Authors needing FFI to external C libs must ensure those libs are present in the container (document the allowlist; default to none).

Output: `target/x86_64-unknown-linux-musl/release/lib<crate_name>.so`

### 11.3 ExaConnection trait (connect-back without linking exarrow-rs)

The problem: the original design returned `exarrow_rs::adbc::Connection` from `ctx.connect_back()`, forcing every UDF that uses connect-back to statically link exarrow-rs. This is expensive and unnecessary — the host process already owns the connection infrastructure.

**Solution:** define an `ExaConnection` trait in `exasol-udf-sdk`. The host runtime implements it using exarrow-rs. The UDF only depends on `exasol-udf-sdk` and `arrow = "58"`.

```rust
// exasol-udf-sdk::connect_back — no exarrow-rs import on the UDF side
pub trait ExaConnection {
    fn query_arrow(&self, sql: &str) -> Result<Vec<RecordBatch>, UdfError>;
    fn execute(&self, sql: &str) -> Result<u64, UdfError>;
    fn import_arrow(&self, table: &str, batches: &[RecordBatch]) -> Result<(), UdfError>;
    fn export_arrow(&self, query: &str) -> Result<Vec<RecordBatch>, UdfError>;
}

pub enum ConnectBackOptions {
    Default,                   // use DB-provided credentials (PB_IMPORT_CONNECTION_INFORMATION)
    Named(String),             // use named Exasol CONNECTION object (e.g. "MY_CONN")
    Explicit {                 // explicit credentials — discouraged; document as last resort
        host: String,
        user: String,
        password: String,
    },
}
```

**New `UdfContext` methods** (replace the old `connect_back` / `connect_back_with`):

```rust
// Lazy default connection — opened on first call, cached for the UDF's lifetime
fn exa(&self) -> Result<&dyn ExaConnection, UdfError>;

// Named Exasol CONNECTION object or explicit options — returns a new connection each call
fn exa_named(&self, name: &str) -> Result<Box<dyn ExaConnection>, UdfError>;
fn exa_connect(&self, opts: ConnectBackOptions) -> Result<Box<dyn ExaConnection>, UdfError>;
```

The `connect-back` Cargo feature flag on `exasol-udf-sdk` remains: when disabled, these methods are absent and the crate has no tokio or exarrow-rs dependency whatsoever.

`exa-udf-runtime` implements `ExaConnection` using `exarrow-rs` under its `CONNECT_BACK_RT` OnceLock runtime. The `HostContextBridge` holds an `Option<Box<dyn ExaConnection>>` for the lazy default and creates new ones on demand.

### 11.4 exapump udf deploy

`exapump` is extended with a `udf` subcommand. The deploy step:

1. Validates the `.so` is a valid ELF with the `__exa_udf_entry` symbol (quick pre-upload sanity check)
2. Uploads the `.so` to the specified BucketFS path (respects `validateservercertificate=0` per project rules)
3. Executes `CREATE OR REPLACE RUST SCALAR SCRIPT` (or `SET SCRIPT` for EMITS UDFs) with the correct `%udf_object` path

```bash
exapump udf deploy \
  --so ./target/x86_64-unknown-linux-musl/release/libmy_udf.so \
  --bucket bfsdefault/default/udfs/ \
  --script my_schema.my_udf \
  [--inputs "x BIGINT, y DOUBLE"]   # optional: inferred from annotation if omitted
  [--outputs "result BIGINT"]       # optional: inferred from annotation if omitted
  [--set]                           # use SET SCRIPT (EMITS) instead of SCALAR SCRIPT
```

**Type inference:** if `cargo exaudf build` detects `#[exasol_udf(input(...), emits(...))]`, it emits a `<crate_name>.udf-meta.json` sidecar alongside the `.so`. `exapump udf deploy` reads this file automatically if present and skips the need for explicit `--inputs` / `--outputs`. Explicit flags always override the sidecar.

### 11.5 UDF author crate template

`cargo exaudf new my-udf` generates:

```toml
# Cargo.toml
[package]
name = "my-udf"
edition = "2024"

[lib]
crate-type = ["cdylib"]

[dependencies]
exasol-udf-sdk = "0.1"
```

```rust
// src/lib.rs
use exasol_udf_sdk::prelude::*;

#[exasol_udf]
pub struct MyUdf;

impl UdfRun for MyUdf {
    fn run(&mut self, ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
        while ctx.next()? {
            let x = ctx.get_i64(0)?.unwrap_or(0);
            ctx.emit(&[Value::Int64(x)])?;
        }
        Ok(())
    }
}
```

Authors add third-party crates to `[dependencies]` freely — they are statically linked into the musl `.so` with no further configuration.

---

## Reference Files

| File | Purpose |
|---|---|
| `~/code/script-languages-release/exaudfclient/exaudfclient.cc` | Launcher contract to replicate (arg format, `HOME=/tmp`, locale, error prefix `F-UDF-CL-*`) |
| `~/code/script-languages-release/flavors/standard-EXASOL-all-python-3.12/flavor_base/build_steps.py` | exaslct task DAG pattern for the Rust flavor |
| `~/code/script-languages-release/flavors/standard-EXASOL-all-python-3.12/flavor_base/build_run/Dockerfile` | Build layer template (swap Bazel for Cargo) |
| `~/code/script-languages-release/flavors/standard-EXASOL-all-python-3.12/flavor_base/language_definitions.json` | JSON schema to replicate for RUST |
| `exarrow-rs` crate (crates.io, v0.12.7) — `src/adbc/connection.rs` | `Connection::from_params`, `query`, `execute_update`, `blocking_import_*`, `blocking_export_*` |
| `exarrow-rs` crate (crates.io, v0.12.7) — `Cargo.toml` | `arrow = "58"` version pin to coordinate with SDK |
| `~/code/script-languages-release/exaudfclient/base/exaudflib/zmqcontainer.proto` | Canonical protocol definition (after `git submodule update --init`) |
