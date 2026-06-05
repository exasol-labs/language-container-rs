# Tasks: add-v1-rust-udf-slim

## Phase 1 — Workspace Bootstrap (Group A)
- [x] 1.1 Download `zmqcontainer.proto` into `crates/exa-proto/proto/zmqcontainer.proto`
- [x] 1.2 Create `PROTO_SOURCES.md` recording URL, commit SHA, and fetch date
- [x] 1.3 Create root `Cargo.toml`: workspace, resolver=2, members, workspace.dependencies, patch.crates-io [expert]
- [x] 1.4 Create `rust-toolchain.toml` pinning channel="1.84" + musl target

## Phase 1 — Workspace Bootstrap (Group B)
- [x] 1.5 Create `crates/exa-proto/{Cargo.toml,build.rs,src/lib.rs}` — prost-build, rerun-if-changed, include! + glob re-export
- [x] 1.6 Create stub crates: exa-zmq-protocol, exasol-udf-sdk, exasol-udf-macros, exa-udf-runtime, exaudfclient, cargo-exaudf
- [x] 1.7 Verify `cargo build -p exa-proto` and `cargo build --workspace` exit 0

## Phase 2 — Protocol Layer (Group C)
- [x] 2.1 Define `messages.rs`/`meta.rs`: ColumnMeta, UdfMeta, IterType, HostEvent, HostAction, ProtocolError
- [x] 2.2 Implement `transport.rs`: ZmqTransport connect/send/recv over DEALER socket
- [x] 2.3 Implement `loop_.rs` Protocol state machine: handshake MT_CLIENT/MT_INFO/MT_META [expert]
- [x] 2.4 Implement scalar/set run loop: MT_RUN, MT_NEXT, MT_EMIT, MT_DONE [expert]
- [x] 2.5 Implement side cases: MT_CLEANUP/MT_FINISHED/MT_CLOSE, MT_PING_PONG, MT_TRY_AGAIN, MT_RESET [expert]
- [x] 2.6 Implement error close path with F-UDF-CL-RUST-#### prefix
- [x] 2.7 Unit tests: prost round-trip, fixture-driven state-machine transitions, column-type mapping

## Phase 3 — SDK + Macro (Group D)
- [x] 3.1 `value.rs`: Value enum + ExaType for eight v1 column types
- [x] 3.2 `context.rs`: UdfRun (defaulted single-call hooks → Unimplemented) and UdfContext traits; error.rs UdfError
- [x] 3.3 `abi.rs`: EXA_UDF_ABI_VERSION, #[repr(C)] ExaUdfVTable
- [x] 3.4 SDK `build.rs`: bake EXA_SDK_FINGERPRINT = "SDK_VERSION:RUSTC_HASH\0"
- [x] 3.5 `exasol-udf-macros`: #[exasol_udf] generates vtable shims, static VT, __exa_udf_entry, catch_unwind, fingerprint embed [expert]
- [x] 3.6 Linker duplicate-symbol guard verified
- [x] 3.7 Unit/trybuild tests: macro expansion compiles; panic→error-code; connect-back feature no-op

## Phase 4 — Host Runtime (Group E)
- [x] 4.1 `loader.rs`: libloading dlopen, resolve __exa_udf_entry, ABI-version + fingerprint gating [expert]
- [x] 4.2 Parse `%udf_object <path>` from script source; route to loader (Option A)
- [x] 4.3 `compiler.rs` stub: JIT returns unsupported-feature error
- [x] 4.4 `rowset.rs` + HostContextBridge: Arrow column builders for eight PB types, typed accessors, NULL handling [expert]
- [x] 4.5 `dispatch.rs`: scalar/set loop wiring bridge next/emit to MT_NEXT/MT_EMIT, emit batching, MT_DONE [expert]
- [x] 4.6 `Runtime::{new,run}`: handshake → meta → resolve → load → dispatch → close; error → destroy
- [x] 4.7 Unit tests: loader ABI/fingerprint mismatch, bridge accessors, dispatch loops

## Phase 5 — Binary (Group F)
- [x] 5.1 `main.rs`: arg parsing (count, lang=rust), HOME=/tmp, stderr tracing, parser-version env override, delegate to Runtime
- [x] 5.2 Error/usage exit codes with F-UDF-CL-RUST- prefix
- [x] 5.3 Unit tests for arg validation and parser-version precedence

## Phase 6 — Slim Container (Group G)
- [x] 6.1 Write root `Dockerfile`: builder rust:1.84-bookworm → runtime debian:12-slim copying binary to /exaudf/exaudfclient
- [x] 6.2 Create `build_info/language_definitions.json` (schema_version 2, RUST alias, lang=rust, executable path)
- [x] 6.3 Build `docker build -t slc-rs-slim:dev .`; smoke-run /exaudf/exaudfclient

## Phase 7 — Test UDFs (Group H)
- [x] 7.1 `test-udfs/scalar-double`: #[exasol_udf], reads i64, emits Int64(x*2); crate-type=["cdylib"]
- [x] 7.2 `test-udfs/set-filter`: #[exasol_udf], loops ctx.next(), emits rows where x>0
- [x] 7.3 `test-udfs/json-parse`: #[exasol_udf] + serde_json, parses string, emits name field
- [x] 7.4 Verify each builds to musl cdylib exporting __exa_udf_entry

## Phase 8 — Integration Tests (Group I)
- [x] 8.1 IT crate scaffold + dev-deps (testcontainers http_wait, reqwest, tokio, exarrow-rs, arrow); integration feature gate
- [x] 8.2 Harness: start exasol/docker-db:2026.1.0 privileged, expose 8563+2581, wait readiness, exarrow-rs connection [expert]
- [x] 8.3 Harness helpers: BucketFS HTTPS PUT; build+load slc-rs-slim:dev; ALTER SESSION SET SCRIPT_LANGUAGES [expert]
- [x] 8.4 IT scenario: scalar double_it(21) → 42
- [x] 8.5 IT scenario: set/EMITS filter_positive → count==positives, all >0
- [x] 8.6 IT scenario: json_field('{"name":"exa"}') → exa
- [x] 8.7 IT scenario: UDF error path surfaces F-UDF-CL-RUST- in SQL error

## Verification
- [x] V.1 cargo build --release exits 0
- [x] V.2 cargo build --release --target x86_64-unknown-linux-musl -p scalar-double -p set-filter -p json-parse exits 0
- [x] V.3 docker build -t slc-rs-slim:dev . exits 0
- [x] V.4 cargo test exits 0 (unit + lib tests)
- [x] V.5 cargo test -p it --features integration exits 0 (all DB scenarios pass)
- [x] V.6 cargo clippy --all-targets --all-features -- -D warnings exits 0
- [x] V.7 cargo fmt --check exits 0
