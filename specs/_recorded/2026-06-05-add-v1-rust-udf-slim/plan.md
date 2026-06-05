# Plan: add-v1-rust-udf-slim

## Summary

Implement v1 of the Rust-native Exasol SLC end-to-end — bootstrap the Cargo workspace, build the full stack (proto → ZMQ protocol → SDK + macro → host runtime → `exaudfclient` binary), package a slim Docker image, and prove it works with running integration tests that start `exasol/docker-db:2026.1.0`, register the SLC, upload precompiled musl `.so` UDFs to BucketFS, call them from SQL, and assert results. This plan supersedes `add-workspace-bootstrap` by folding the workspace bootstrap in as Phase 1.

## Design

### Context

The repository is an empty git repo (only `CLAUDE.md` and `specs/`). The technical design (`specs/references/design.md`) defines a layered, acyclic crate graph and the `localzmq+protobuf` wire protocol. v1 must deliver a working slice: precompiled-`.so` (Option A) scalar and set/EMITS UDFs running against a real DB, with a third-party dependency statically linked into a musl artifact. Connect-back, JIT (Option C), single-call `SC_FN_*`, and the `cargo-exaudf` CLI are deferred.

- **Goals**
  - A fully building workspace with `exa-proto` generating prost bindings.
  - A pure, unit-testable protocol state machine covering the v1 message set.
  - An SDK + `#[exasol_udf]` macro that an author links to produce a loadable cdylib.
  - A host runtime that dlopen-loads a fingerprint-checked `.so` and drives scalar/set dispatch with Arrow buffering.
  - A slim Docker image shipping `/exaudf/exaudfclient`.
  - Integration tests that actually start Exasol and prove scalar, set/EMITS, and a `serde_json`-using UDF return correct results.

- **Non-Goals**
  - Connect-back (`ExaConnection`, `exa()`/`exa_named()`/`exa_connect()`), tokio runtime — out of v1.
  - JIT / Option C compilation — runtime returns an unsupported error.
  - Single-call `SC_FN_*` (default output columns, import/export spec) and virtual-schema adapter calls.
  - `cargo-exaudf` CLI functionality and `exapump udf deploy` (stub crate only).
  - Typed schema annotations `#[exasol_udf(input(...), emits(...))]`.
  - The `jit` (~1.4 GB) image and the exaslct flavor DAG.

### Decision

#### Architecture

```
                                exaudfclient (binary)
                                        │
                                        ▼
                              exa-udf-runtime (host)
                          ┌─────────────┴───────────────┐
                          ▼                              ▼
                  exa-zmq-protocol               exasol-udf-sdk ──▶ exasol-udf-macros
                          │                              │
                          ▼                              ▼  (ABI: __exa_udf_entry)
                      exa-proto                  test-udfs/*.so (musl cdylib)
                                                         │
   Integration harness:  testcontainers ──▶ exasol/docker-db:2026.1.0
        exarrow-rs (SQL) ──┐         BucketFS HTTP PUT ──┘  slc-rs-slim:dev image
                           └──▶ SELECT udf(...) ─── assert
```

Data flow per UDF call: DB ROUTER → DEALER transport → `Protocol` state machine (`ExascriptResponse` → `HostEvent`) → `exa-udf-runtime` dispatch → `HostContextBridge` (`UdfContext`) → user `.so` via vtable → `emit` → `HostAction` → `Protocol` (`ExascriptRequest`) → transport → DB.

#### Patterns

| Pattern | Where | Why |
|---------|-------|-----|
| Pure state machine, no I/O | `exa-zmq-protocol::Protocol` | Unit-testable with fixtures; socket lives only in `ZmqTransport` |
| Single C-ABI crossing | `__exa_udf_entry` → `ExaUdfVTable` | Rich traits stay host-side; only `#[repr(C)]` vtable crosses |
| ABI version + fingerprint gate | `loader.rs` at dlopen | Clear error instead of UB on toolchain mismatch (Option A) |
| `catch_unwind` in run shim | `#[exasol_udf]` macro | Convert UDF panics to error codes, no unwind across FFI |
| Linker duplicate-symbol guard | `#[exasol_udf]` macro | Two annotations → duplicate `__exa_udf_entry` → link error |
| Arrow column builders + emit batching | `HostContextBridge` | Column-oriented proto maps to Arrow; avoids row-by-row alloc |
| Static musl linking | `test-udfs/*` | Self-contained `.so`, no glibc/system-lib needs in slim image |
| testcontainers privileged + ephemeral ports | integration harness | CI-friendly, self-contained real-DB proof |

### Consequences

| Decision | Alternatives Considered | Rationale |
|----------|------------------------|-----------|
| Fold workspace bootstrap into Phase 1; supersede `add-workspace-bootstrap` | Implement bootstrap plan separately first | User asked to bootstrap "along the way"; one atomic v1 plan avoids a stale half-applied bootstrap plan |
| v1 = Option A (precompiled) only; JIT returns unsupported | Implement JIT too | JIT needs the ~1.4 GB image + vendored registry; Option A proves the full protocol/SDK/loader path with far less surface |
| Exclude connect-back from v1 entirely | Keep `connect-back` feature wired | Connect-back pulls in tokio + exarrow-rs and a whole credential path; not needed to prove scalar/set/3rd-party-dep |
| testcontainers-rs + `exasol/docker-db:2026.1.0`, privileged | Manual docker-compose; emulator | Self-contained, CI-friendly, RAII teardown; emulator can't prove BucketFS + SQL path |
| Pin DB image to `2026.1.0` (not `2026.latest`) | Float to `2026.latest` per CLAUDE.md | Reproducible ITs; `2026.1.0` is the confirmed latest 2026.x |
| `[patch.crates-io]` for exarrow-rs in root despite v1 not using connect-back | Add later | Keeps the workspace manifest stable for future phases; harness uses exarrow-rs directly for SQL assertions |
| musl `.so` built per-crate via `cargo build --target` in the IT setup | `cargo-exaudf build` wrapper | `cargo-exaudf` is out of v1 scope; raw `cargo build --target x86_64-unknown-linux-musl` is sufficient |

## Features

| Feature | Status | Spec |
|---------|--------|------|
| workspace-bootstrap | NEW | `specs/_plans/add-v1-rust-udf-slim/workspace/bootstrap/spec.md` |
| wire-protocol | NEW | `specs/_plans/add-v1-rust-udf-slim/protocol/wire-protocol/spec.md` |
| udf-sdk | NEW | `specs/_plans/add-v1-rust-udf-slim/sdk/udf-sdk/spec.md` |
| host-dispatch | NEW | `specs/_plans/add-v1-rust-udf-slim/runtime/host-dispatch/spec.md` |
| launcher | NEW | `specs/_plans/add-v1-rust-udf-slim/binary/launcher/spec.md` |
| slim-image | NEW | `specs/_plans/add-v1-rust-udf-slim/container/slim-image/spec.md` |
| db-roundtrip | NEW | `specs/_plans/add-v1-rust-udf-slim/integration/db-roundtrip/spec.md` |
| test-udfs | NEW | `specs/_plans/add-v1-rust-udf-slim/examples/test-udfs/spec.md` |

## Dependencies

- `exarrow-rs` at `/home/talos/code/exarrow-rs` (v0.12.5; `arrow = "58"`) — present; used by the IT harness for SQL and patched into the workspace.
- Network access to `raw.githubusercontent.com` (one-time proto fetch).
- Build host: `libzmq3-dev`/`libzmq` + headers, `protobuf-compiler`, `pkg-config`; `rustup` musl target `x86_64-unknown-linux-musl`.
- A working Docker daemon (privileged containers allowed) for the slim image build and the integration tests; `exasol/docker-db:2026.1.0` pullable.
- `testcontainers = { version, features = ["http_wait"] }`, `reqwest` (BucketFS upload), `tokio` (test runtime) — dev-dependencies of the IT crate only.

## Implementation Tasks

### Phase 1 — Workspace bootstrap (supersedes add-workspace-bootstrap)

- [ ] 1.1 Download `zmqcontainer.proto` from the GitHub raw URL into `crates/exa-proto/proto/zmqcontainer.proto`
- [ ] 1.2 Create `PROTO_SOURCES.md` recording URL, commit SHA `75dc742299d3bfa5fb1d6e587097984017868364`, and fetch date
- [ ] 1.3 Create root `Cargo.toml`: `[workspace]`, `resolver = "2"`, members (seven crates + `test-udfs/*`), `[workspace.dependencies]`, `[patch.crates-io] exarrow-rs` [expert]
- [ ] 1.4 Create `rust-toolchain.toml` pinning `channel = "1.84"` (and `targets = ["x86_64-unknown-linux-musl"]`)
- [ ] 1.5 Create `crates/exa-proto/{Cargo.toml, build.rs, src/lib.rs}` — prost-build, `rerun-if-changed`, `include!` + glob re-export
- [ ] 1.6 Create stub crates: `exa-zmq-protocol`, `exasol-udf-sdk`, `exasol-udf-macros` (proc-macro), `exa-udf-runtime`, `exaudfclient` (bin), `cargo-exaudf` (bin)
- [ ] 1.7 Verify `cargo build -p exa-proto` and `cargo build --workspace` exit 0

### Phase 2 — Protocol layer (exa-zmq-protocol)

- [ ] 2.1 Define `messages.rs` / `meta.rs`: `ColumnMeta`, `UdfMeta`, `IterType`, `HostEvent`, `HostAction`, `ProtocolError`
- [ ] 2.2 Implement `transport.rs`: `ZmqTransport::{connect, send, recv}` over a DEALER socket (one frame per message)
- [ ] 2.3 Implement `loop_.rs` `Protocol` state machine: handshake (MT_CLIENT/MT_INFO/MT_META), phase tracking [expert]
- [ ] 2.4 Implement scalar/set run loop: MT_RUN, MT_NEXT, MT_EMIT, MT_DONE; input-exhaustion signal [expert]
- [ ] 2.5 Implement side cases: MT_CLEANUP/MT_FINISHED/MT_CLOSE close sequence, MT_PING_PONG echo, MT_TRY_AGAIN, MT_RESET [expert]
- [ ] 2.6 Implement error close path with `F-UDF-CL-RUST-####` prefix; unexpected-message → `ProtocolError`
- [ ] 2.7 Unit tests: prost round-trip per message type; fixture-driven state-machine transitions; column-type mapping

### Phase 3 — SDK + macro (exasol-udf-sdk, exasol-udf-macros)

- [ ] 3.1 `value.rs`: `Value` enum + `ExaType` covering the eight v1 column types
- [ ] 3.2 `context.rs`: `UdfRun` (with defaulted single-call hooks returning `Unimplemented`) and `UdfContext` traits; `error.rs` `UdfError`
- [ ] 3.3 `abi.rs`: `EXA_UDF_ABI_VERSION`, `#[repr(C)] ExaUdfVTable`
- [ ] 3.4 SDK `build.rs`: bake `EXA_SDK_FINGERPRINT` = `"SDK_VERSION:RUSTC_HASH\0"`
- [ ] 3.5 `exasol-udf-macros`: `#[exasol_udf]` generates vtable shims, static VT, `__exa_udf_entry`, `catch_unwind` run shim, fingerprint embed [expert]
- [ ] 3.6 Linker duplicate-symbol guard verified (two annotations → link error)
- [ ] 3.7 Unit/trybuild tests: macro expansion compiles; panic→error-code; declare `connect-back` feature as no-op in v1

### Phase 4 — Host runtime (exa-udf-runtime)

- [ ] 4.1 `loader.rs`: `libloading` dlopen, resolve `__exa_udf_entry`, ABI-version + fingerprint gating, `create`/`destroy`, hold `Library` [expert]
- [ ] 4.2 Parse `%udf_object <path>` from script source; route to loader (Option A)
- [ ] 4.3 `compiler.rs` stub: JIT path returns an unsupported-feature error
- [ ] 4.4 `rowset.rs` + `HostContextBridge`: Arrow column builders for the eight PB types, typed accessors, NULL handling [expert]
- [ ] 4.5 `dispatch.rs`: scalar/set loop wiring bridge `next`/`emit` to MT_NEXT/MT_EMIT, emit batching, MT_DONE [expert]
- [ ] 4.6 `Runtime::{new, run}`: handshake → meta → resolve artifact → load → dispatch → close; error → close path + `destroy`
- [ ] 4.7 Unit tests: loader ABI/fingerprint mismatch (built fixture `.so`), bridge accessors against fake transport, dispatch loops

### Phase 5 — Binary (exaudfclient)

- [ ] 5.1 `main.rs`: arg parsing (count, `lang=rust`), `HOME=/tmp`, stderr tracing, parser-version env override, delegate to `Runtime`
- [ ] 5.2 Error/usage exit codes with `F-UDF-CL-RUST-` prefix
- [ ] 5.3 Unit tests for arg validation and parser-version precedence

### Phase 6 — Slim container

- [ ] 6.1 Write root `Dockerfile`: builder `rust:1.84-bookworm` (libzmq3-dev, protobuf-compiler, pkg-config) → `cargo build --release -p exaudfclient`; runtime `debian:12-slim` (libzmq5, ca-certificates, locales, UTF-8) copying binary to `/exaudf/exaudfclient`
- [ ] 6.2 Create `build_info/language_definitions.json` (schema_version 2, RUST alias, lang=rust, executable path)
- [ ] 6.3 Build task: `docker build -t slc-rs-slim:dev .`; smoke-run `/exaudf/exaudfclient` (usage + non-zero exit)

### Phase 7 — Test UDF crates (test-udfs)

- [ ] 7.1 `test-udfs/scalar-double`: `#[exasol_udf]`, reads i64, emits `Int64(x*2)`; `crate-type = ["cdylib"]`
- [ ] 7.2 `test-udfs/set-filter`: `#[exasol_udf]`, loops `ctx.next()`, emits rows where `x > 0`
- [ ] 7.3 `test-udfs/json-parse`: `#[exasol_udf]` + `serde_json`, parses string column, emits `name` field
- [ ] 7.4 Verify each builds to a musl cdylib exporting `__exa_udf_entry` via `cargo build --release --target x86_64-unknown-linux-musl -p <crate>`

### Phase 8 — Integration tests (crates/it or tests/)

- [ ] 8.1 IT crate scaffold + dev-deps (`testcontainers` http_wait, `reqwest`, `tokio`, `exarrow-rs`, `arrow`); gate behind `integration` feature
- [ ] 8.2 Harness: start `exasol/docker-db:2026.1.0` privileged, expose 8563 + 2580, wait for readiness, build `exarrow-rs` connection (`validate_server_certificate(false)`) [expert]
- [ ] 8.3 Harness helpers: BucketFS HTTP PUT upload; build + load `slc-rs-slim:dev` as the language container; `ALTER SESSION SET SCRIPT_LANGUAGES` [expert]
- [ ] 8.4 IT scenario: scalar `double_it(21)` → 42
- [ ] 8.5 IT scenario: set/EMITS `filter_positive` over a mixed table → count == positives, all emitted > 0
- [ ] 8.6 IT scenario: `json_field('{"name":"exa"}')` → `exa` (serde_json statically linked)
- [ ] 8.7 IT scenario: UDF error path surfaces `F-UDF-CL-RUST-` in the SQL error

## Parallelization

| Parallel Group | Tasks |
|----------------|-------|
| Group A — proto + manifest | 1.1, 1.2, then 1.3, 1.4 |
| Group B — exa-proto + stubs | 1.5, 1.6 → 1.7 |
| Group C — protocol | 2.1–2.7 |
| Group D — SDK + macro | 3.1–3.7 |
| Group E — runtime | 4.1–4.7 |
| Group F — binary | 5.1–5.3 |
| Group G — container | 6.1–6.3 |
| Group H — test UDFs | 7.1–7.4 |
| Group I — integration | 8.1–8.7 |

Sequential dependencies:
- Group A → Group B → (C, D)
- C, D → E (runtime needs protocol + SDK)
- E → F (binary delegates to runtime)
- D → H (test UDFs need SDK + macro)
- F → G (image ships the binary)
- G, H → I (ITs need the image and the musl `.so` artifacts)

Within phases, Groups C and D can run concurrently after B. Groups F, G follow E; H follows D and can run concurrently with E/F/G; I is last.

## Dead Code Removal

| Type | Location | Reason |
|------|----------|--------|
| Plan | `specs/_plans/add-workspace-bootstrap/` | Superseded — its workspace + exa-proto scope is folded into Phase 1 of this plan; remove only after this plan's Phase 1 is recorded, to avoid a stale duplicate bootstrap |

No source code exists yet (greenfield); nothing else to remove.

## Verification

### Scenario Coverage

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| workspace: Cargo.toml well-formed | Integration | `crates/exa-proto/tests/build_smoke.rs` | `workspace_manifest_well_formed` |
| workspace: toolchain pinned | Integration | `crates/exa-proto/tests/build_smoke.rs` | `toolchain_channel_is_1_84` |
| workspace: seven stubs compile | Integration | `crates/exa-proto/tests/build_smoke.rs` | `all_stubs_compile` |
| workspace: vendors zmqcontainer.proto | Integration | `crates/exa-proto/tests/build_smoke.rs` | `proto_file_present_and_nonempty` |
| workspace: build.rs generates bindings | Integration | `crates/exa-proto/tests/build_smoke.rs` | `generated_request_response_types` |
| workspace: lib.rs re-exports types | Integration | `crates/exa-proto/tests/build_smoke.rs` | `exascript_request_usable` |
| protocol: DEALER transport connects | Integration | `crates/exa-zmq-protocol/tests/transport.rs` | `transport_connects_to_ipc` |
| protocol: round-trips one frame each | Integration | `crates/exa-zmq-protocol/tests/transport.rs` | `transport_round_trip_single_frame` |
| protocol: handshake Info then Meta | Unit | `crates/exa-zmq-protocol/src/loop_.rs` | `handshake_emits_info_then_meta` |
| protocol: metadata maps column types | Unit | `crates/exa-zmq-protocol/src/meta.rs` | `meta_maps_all_pb_types` |
| protocol: scalar run loop to DONE | Unit | `crates/exa-zmq-protocol/src/loop_.rs` | `scalar_loop_next_emit_done` |
| protocol: set/EMITS multiple batches | Unit | `crates/exa-zmq-protocol/src/loop_.rs` | `set_loop_multiple_batches` |
| protocol: close sequence after DONE | Unit | `crates/exa-zmq-protocol/src/loop_.rs` | `close_sequence_cleanup_finished_close` |
| protocol: ping-pong echoed | Unit | `crates/exa-zmq-protocol/src/loop_.rs` | `ping_pong_echoes` |
| protocol: reset restarts iteration | Unit | `crates/exa-zmq-protocol/src/loop_.rs` | `reset_restarts_iteration` |
| protocol: try-again surfaced | Unit | `crates/exa-zmq-protocol/src/loop_.rs` | `try_again_no_phase_advance` |
| protocol: unexpected message errors | Unit | `crates/exa-zmq-protocol/src/loop_.rs` | `unexpected_message_is_error` |
| protocol: error close path prefix | Unit | `crates/exa-zmq-protocol/src/loop_.rs` | `error_close_path_prefix` |
| sdk: Value/ExaType cover v1 types | Unit | `crates/exasol-udf-sdk/src/value.rs` | `value_exatype_variants` |
| sdk: UdfContext typed accessors API | Integration | `crates/exa-udf-runtime/tests/bridge.rs` | `bridge_typed_accessors` |
| sdk: UdfRun default hooks Unimplemented | Unit | `crates/exasol-udf-sdk/src/context.rs` | `default_hooks_unimplemented` |
| sdk: ABI constants/vtable layout | Unit | `crates/exasol-udf-sdk/src/abi.rs` | `abi_version_and_vtable_layout` |
| sdk: fingerprint baked at build | Unit | `crates/exasol-udf-sdk/src/abi.rs` | `fingerprint_baked_nonempty` |
| sdk: macro generates entry + vtable | Integration | `crates/exasol-udf-macros/tests/trybuild.rs` | `macro_generates_entry` |
| sdk: run shim catches panics | Integration | `crates/exa-udf-runtime/tests/loader.rs` | `panicking_udf_returns_error_code` |
| sdk: two annotations fail to link | Integration | `crates/exasol-udf-macros/tests/trybuild/dup_entry.rs` | `duplicate_entry_link_error` |
| runtime: loader accepts matching .so | Integration | `crates/exa-udf-runtime/tests/loader.rs` | `loader_accepts_matching_so` |
| runtime: loader rejects ABI mismatch | Integration | `crates/exa-udf-runtime/tests/loader.rs` | `loader_rejects_abi_mismatch` |
| runtime: loader rejects fingerprint mismatch | Integration | `crates/exa-udf-runtime/tests/loader.rs` | `loader_rejects_fingerprint_mismatch` |
| runtime: parses %udf_object path | Unit | `crates/exa-udf-runtime/src/lib.rs` | `parses_udf_object_path` |
| runtime: JIT unsupported in v1 | Unit | `crates/exa-udf-runtime/src/compiler.rs` | `jit_returns_unsupported` |
| runtime: bridge materializes rows | Integration | `crates/exa-udf-runtime/tests/bridge.rs` | `bridge_materializes_input_rows` |
| runtime: scalar dispatch emits batch | Integration | `crates/exa-udf-runtime/tests/dispatch.rs` | `scalar_dispatch_emits_one_batch` |
| runtime: set dispatch multiple rows | Integration | `crates/exa-udf-runtime/tests/dispatch.rs` | `set_dispatch_batched_emit` |
| runtime: UDF error closes with prefix | Integration | `crates/exa-udf-runtime/tests/dispatch.rs` | `udf_error_close_prefix` |
| launcher: valid invocation delegates | Integration | `crates/exaudfclient/tests/cli.rs` | `valid_invocation_delegates` |
| launcher: wrong arg count rejected | Integration | `crates/exaudfclient/tests/cli.rs` | `wrong_arg_count_rejected` |
| launcher: unsupported lang rejected | Integration | `crates/exaudfclient/tests/cli.rs` | `unsupported_lang_rejected` |
| launcher: env overrides parser version | Unit | `crates/exaudfclient/src/main.rs` | `env_overrides_parser_version` |
| launcher: runtime failure prefixed | Integration | `crates/exaudfclient/tests/cli.rs` | `runtime_failure_prefixed` |
| slim-image: docker build produces image | Integration | `crates/it/tests/container.rs` | `slim_image_builds_with_binary` |
| slim-image: builder toolchain matches | Integration | `crates/it/tests/container.rs` | `builder_toolchain_is_1_84` |
| slim-image: runtime slim/self-sufficient | Integration | `crates/it/tests/container.rs` | `runtime_has_no_toolchain` |
| slim-image: language_definitions present | Integration | `crates/it/tests/container.rs` | `language_definitions_well_formed` |
| slim-image: binary reports usage | Integration | `crates/it/tests/container.rs` | `binary_prints_usage` |
| test-udfs: scalar-double emits 2x | Integration | `crates/it/tests/db_roundtrip.rs` | `scalar_double_returns_42` |
| test-udfs: set-filter positive only | Integration | `crates/it/tests/db_roundtrip.rs` | `set_filter_emits_positive_only` |
| test-udfs: json-parse extracts field | Integration | `crates/it/tests/db_roundtrip.rs` | `json_parse_extracts_name` |
| test-udfs: musl .so builds | Integration | `crates/it/tests/container.rs` | `test_udfs_build_musl_cdylib` |
| db-roundtrip: harness starts + connects | Integration | `crates/it/tests/db_roundtrip.rs` | `harness_starts_and_connects` |
| db-roundtrip: SLC registered | Integration | `crates/it/tests/db_roundtrip.rs` | `slc_registered_for_session` |
| db-roundtrip: artifact uploaded to BucketFS | Integration | `crates/it/tests/db_roundtrip.rs` | `udf_uploaded_to_bucketfs` |
| db-roundtrip: scalar doubles BIGINT | Integration | `crates/it/tests/db_roundtrip.rs` | `scalar_double_returns_42` |
| db-roundtrip: set/EMITS filters rows | Integration | `crates/it/tests/db_roundtrip.rs` | `set_filter_emits_positive_only` |
| db-roundtrip: 3rd-party dep linked | Integration | `crates/it/tests/db_roundtrip.rs` | `json_parse_extracts_name` |
| db-roundtrip: UDF error surfaces prefix | Integration | `crates/it/tests/db_roundtrip.rs` | `udf_error_surfaces_prefix` |

### Manual Testing

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| workspace-bootstrap | `cargo build --workspace` | Exit 0; all seven crates compile |
| wire-protocol | `cargo test -p exa-zmq-protocol` | 0 failures; state-machine + round-trip tests pass |
| udf-sdk | `cargo test -p exasol-udf-sdk -p exasol-udf-macros` | 0 failures; trybuild dup-entry case fails to compile as expected |
| host-dispatch | `cargo test -p exa-udf-runtime` | 0 failures; loader ABI/fingerprint rejection tests pass |
| launcher | `cargo run -p exaudfclient` | Prints usage referencing `lang=rust`; non-zero exit |
| test-udfs | `cargo build --release --target x86_64-unknown-linux-musl -p scalar-double -p set-filter -p json-parse` | Three `lib*.so` under `target/x86_64-unknown-linux-musl/release/` |
| slim-image | `docker build -t slc-rs-slim:dev . && docker run --rm slc-rs-slim:dev /exaudf/exaudfclient` | Image builds; container prints usage, exits non-zero |
| db-roundtrip | `cargo test -p it --features integration -- --nocapture` | Exasol container starts; scalar→42, set count matches positives, json→`exa`; all green |

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Build | `cargo build --release` | Exit 0 |
| Build musl UDFs | `cargo build --release --target x86_64-unknown-linux-musl -p scalar-double -p set-filter -p json-parse` | Exit 0; three `.so` artifacts |
| Build image | `docker build -t slc-rs-slim:dev .` | Exit 0 |
| Test (unit + lib) | `cargo test` | 0 failures |
| Test (integration, real DB) | `cargo test -p it --features integration` | 0 failures; Exasol starts and all UDF scenarios pass |
| Lint | `cargo clippy --all-targets --all-features -- -D warnings` | 0 errors/warnings |
| Format | `cargo fmt --check` | No changes |
