# Plan: add-v2-rust-udf-complete

## Summary

Completes the Rust SLC by adding the four capability areas v1 left stubbed — single-call protocol (`SC_FN_*` incl. virtual-schema adapter), connect-back via an `ExaConnection` trait backed by exarrow-rs, typed `#[exasol_udf(input(...), emits(...))]` macro annotations, and the `cargo exaudf` CLI (`new`/`build`/`validate`) — so authors can scaffold, build, validate, deploy, and run fully-featured Rust UDFs.

## Design

### Context

v1 shipped the wire protocol, scalar/set dispatch, ABI loader, slim container, and test UDFs, but four capabilities are declared-but-empty. The proto already carries `MtCall`/`MtReturn`/`MtUndefinedCall` and the `SingleCallFunctionId` enum, the workspace already has the exarrow-rs path dep, tokio, and `arrow = "58"`, and `UdfMeta` already parses `single_call_mode` — but no code acts on any of it. This plan wires the existing scaffolding into working features without re-architecting v1.

- **Goals** — make single-call (including VS adapter), connect-back, typed annotations, and the `cargo exaudf` CLI fully functional and tested end-to-end against a live Exasol container.
- **Non-Goals** — JIT compilation (`compiler.rs` stays returning `UnsupportedFeature`); `exapump udf deploy` implementation (out of scope; only the schema sidecar that feeds it is in scope); new column types beyond the v1 set.

### Decision

#### Architecture

```
DB ──MT_INFO(conn info)──▶ Protocol ──HostEvent::Info{conn}──▶ Runtime
DB ──MT_CALL(SC_FN_*)────▶ Protocol ──HostEvent::SingleCall──▶ single_call.rs
                                                                  │
                          ┌───────────────────────────────────────┤ route by SingleCallFn
                          ▼                                        ▼
                  vtable.default_output_columns      vtable.virtual_schema_adapter_call
                  vtable.generate_sql_import/export        (etc.)
                          │                                        │
        SingleCallReturn / UndefinedCall ──▶ Protocol ──MT_RETURN / MT_UNDEFINED_CALL──▶ DB

UDF ──ctx.exa()──▶ HostContextBridge ──▶ dyn ExaConnection (impl in runtime)
                                              │ block_on(CONNECT_BACK_RT)
                                              ▼
                                        exarrow-rs Connection (query/execute/import/export)
```

The UDF side depends only on `exasol-udf-sdk` + `arrow`. The `ExaConnection` trait lives in the SDK (behind the `connect-back` feature); the runtime owns the only exarrow-rs link and the `CONNECT_BACK_RT` `OnceLock<Runtime>` (current_thread).

#### Patterns

| Pattern | Where | Why |
|---------|-------|-----|
| Pure state machine emits events / consumes actions | `exa-zmq-protocol` MT_CALL handling | Keeps I/O-free unit-testability invariant from v1 |
| Trait in SDK, impl in runtime | `ExaConnection` | UDFs avoid statically linking exarrow-rs (design §11.3) |
| Dedicated `OnceLock<Runtime>` block_on bridge | runtime `connect_back.rs` | Sync ZMQ loop calls async exarrow-rs without entering an async context |
| Cargo subcommand convention | `cargo-exaudf` binary | `cargo exaudf <cmd>` dispatch, musl triple hidden |
| Compile-time schema mapping | `exasol-udf-macros` annotation parser | Map Rust type tokens to `ExaType`, embed in vtable |

### Consequences

| Decision | Alternatives Considered | Rationale |
|----------|------------------------|-----------|
| `ExaConnection` trait in SDK, runtime implements it | Return `exarrow_rs::adbc::Connection` directly | Avoids forcing every connect-back UDF to statically link exarrow-rs (design §11.3) |
| Wire `SC_FN_VIRTUAL_SCHEMA_ADAPTER_CALL` to a real hook | `MT_UNDEFINED_CALL` stub | User explicitly requested wiring it like the other SC_FN_* hooks |
| Skip JIT entirely | Implement Option C compiler | User deferred JIT; keep Option A (precompiled `.so`) only |
| VS adapter verified at unit level only | Full DB roundtrip for VS adapter | VS adapter needs no live DB; unit dispatch test is sufficient |
| Annotation type errors at compile time | Load-time-only validation | Catch unmappable types early; runtime still validates count/type vs DB metadata |

## v2 Completion — Connect-Back Root Cause and Fix

> Augments this plan. The original Groups A–E and V.1–V.5 are implemented and verified.
> The single remaining gap is V.6: the two connect-back integration scenarios crash the
> Exasol session on `2026.1.0`. This section absorbs the `connect-back-fix` child plan and
> adds the work to root-cause and close that gap, bump to `0.2.0`, and land the result on a
> `v2` branch.

### Absorbed: connect-back-fix (already implemented)

The `connect-back-fix` plan is folded in here. Its two fixes are already in the tree and
are treated as done context, not rework:

1. **Feature flag** — `crates/exaudfclient/Cargo.toml` enables `features = ["connect-back"]`
   on `exa-udf-runtime`, so the runtime's `dyn UdfContext` vtable layout matches the `.so`
   compiled with connect-back (previously a layout mismatch → process crash).
2. **Run-phase `ConnInfo`** — `dispatch.rs` captures `HostEvent::ConnInfo(ci)` during the run
   phase and feeds it to `HostContextBridge` via an on-demand `conn_requester` closure that
   sends `MT_IMPORT` (`PB_IMPORT_CONNECTION_INFORMATION`) while the dispatch loop is blocked
   on the function return.

After these fixes `ctx.exa()` reaches credentials, but opening the connect-back connection
over `transport=websocket` triggers a server-side SIGABRT on `2026.1.0`.

### Problem

The current `connect_back.rs` pins `transport=websocket` and points the test `CB_SELF`
connection at the container's own `eth0` IP. On `2026.1.0` the DB creates a new connect-back
session process and then crashes the main session handler with signal 6 (`peer closed
connection without sending TLS close_notify`). The prior conclusion ("known server bug") was
not root-caused against how the reference (Python) SLC actually does connect-back.

### Root Cause (from the reference C++/Python SLC)

Evidence comes from `exasol/script-languages` (`exaudfclient/base/exaudflib/zmqcontainer.proto`
and the language `get_connection` tests):

1. **There is no proactive handshake credential and no dedicated "connect-back proxy."**
   `exascript_info` (`MT_INFO`) carries no `connection_information` field. The *only* way the
   client obtains credentials is `MT_IMPORT` with `kind = PB_IMPORT_CONNECTION_INFORMATION`
   naming a `CONNECTION` object — which returns `connection_information_rep { kind, address,
   user, password }`. Our "proactive ConnInfo from the handshake" path is therefore a phantom;
   the on-demand `MT_IMPORT` path is the real and only one.
2. **The SLC never opens the connection itself.** `exa.get_connection(name)` (Python) and
   `ExaConnectionInformationImpl` (Java) are pure metadata pass-throughs: they hand the UDF
   the `kind`/`address`/`user`/`password` literally as written in `CREATE CONNECTION ... TO
   '<address>' USER '<user>' IDENTIFIED BY '<password>'`. The Python SLC "connect-back" that
   works on 2025 and 2026 is just the *user's own code* calling `pyexasol.connect(dsn=address,
   …)` against an ordinary, routable Exasol endpoint.
3. **`address` is an arbitrary literal, authenticated with user/password — never a token.**
   `connection_information_rep` has no token field; `kind` is `password`.

Conclusion: the crash is not a "WebSocket vs native" server bug. It is caused by treating the
named connection as an internal connect-back proxy and pointing it at the UDF sandbox's own
loopback/eth0 `:8563`. The correct mechanism is to connect to the `address` from the named
connection exactly as an external client would, authenticating with the returned user/password.

### Fix Design

| Decision | Detail |
|----------|--------|
| Connect like an external client | Open the connection to the `connection_information_rep.address` with its `user`/`password`; make no assumption of a special proxy endpoint. |
| Use the native binary protocol as the connect-back transport | The connect-back DSN MUST use the exarrow-rs native binary protocol (the default `native` feature), not WebSocket. Removing the `transport=websocket` override is sufficient since `native` is the exarrow-rs default. Native is faster and matches the main-session transport; verify it connects cleanly against a routable `2026.1.0` endpoint. See ADR in `decision-log.md`. |
| Correct the test connection address | `CB_SELF` MUST be created `TO '<routable-endpoint>:8563'` reachable from the UDF sandbox, not the container's own loopback/eth0 that triggered the SIGABRT. |
| Keep the on-demand `MT_IMPORT` path as primary | Since the handshake carries no credentials, the `conn_requester` (`MT_IMPORT`) path is the supported path; proactive credentials remain a no-op fallback only if a future server pushes them. |
| Strip debug side-channel | Remove the `/tmp/cb_debug.txt` `debug_write` instrumentation from `connect_back.rs` before release. |

## Features

| Feature | Status | Spec |
|---------|--------|------|
| protocol/wire-protocol | CHANGED | `protocol/wire-protocol/spec.md` |
| runtime/host-dispatch | CHANGED | `runtime/host-dispatch/spec.md` |
| sdk/udf-sdk | CHANGED | `sdk/udf-sdk/spec.md` |
| integration/db-roundtrip | CHANGED | `integration/db-roundtrip/spec.md` |
| examples/test-udfs | CHANGED | `examples/test-udfs/spec.md` |
| tools/cargo-exaudf | NEW | `tools/cargo-exaudf/spec.md` |

## Dependencies

- `exarrow-rs` v0.12.5 (path dep + `[patch.crates-io]`, already in workspace `Cargo.toml`)
- `tokio` with `full` features (already present) — used only by the runtime `CONNECT_BACK_RT`
- `arrow = "58"` (already pinned, shared across SDK and exarrow-rs)
- `rustup` available on the author host for `cargo exaudf build` musl target install

## Implementation Tasks

### Group A — Protocol layer (foundation)

- [ ] 1.1 Add `MtCall`/`MtReturn`/`MtUndefinedCall` handling and `SingleCallFn` decoding in `exa-zmq-protocol/src/loop_.rs`
- [ ] 1.2 Add `HostEvent::SingleCall` and `HostAction::SingleCallReturn` / `HostAction::UndefinedCall` to `messages.rs`
- [ ] 1.3 Surface `ExascriptConnectionInformationRep` (host/port/user/password) on `HostEvent::Info`
- [ ] 1.4 Treat `MT_CALL` arriving in a scalar/set run phase as a `ProtocolError`
- [ ] 1.5 Unit tests for single-call request/return/undefined and conn-info parsing in `exa-zmq-protocol/tests/`

### Group B — SDK surface (depends on A for event shapes only at integration; can start in parallel)

- [ ] 2.1 Add `virtual_schema_adapter_call` to the `UdfRun` trait (default `Unimplemented`) and the `ExaUdfVTable` in `context.rs` / `abi.rs`
- [ ] 2.2 Define `ExaConnection` trait + `ConnectBackOptions` enum in new `exasol-udf-sdk/src/connect_back.rs`, gated by the `connect-back` feature [expert]
- [ ] 2.3 Add `exa` / `exa_named` / `exa_connect` methods to `UdfContext` behind the `connect-back` feature; confirm absence (and no tokio/exarrow dep) when disabled
- [ ] 2.4 Extend `exasol-udf-macros` to parse `input(...)` / `emits(...)`, map Rust types to `ExaType`, embed schema in the vtable, and emit a compile error on unmappable types [expert]
- [ ] 2.5 Add macro unit + `trybuild` compile-fail tests for annotation parsing and unknown-type errors

### Group C — Runtime wiring (depends on A and B)

- [ ] 3.1 Add `exa-udf-runtime/src/single_call.rs` routing `HostEvent::SingleCall` → matching vtable hook → `SingleCallReturn` / `UndefinedCall`
- [ ] 3.2 In `dispatch.rs`, detect `single_call_mode` from `MT_META` and route to the single-call dispatcher instead of the run loop
- [ ] 3.3 Add `exa-udf-runtime/src/connect_back.rs`: `CONNECT_BACK_RT` `OnceLock<Runtime>`, implement `ExaConnection` over exarrow-rs (`Connection::from_params`, `query`, `execute_update`, `blocking_import_from_record_batch`, `blocking_export_to_record_batches`), mapping errors to `UdfError::ConnectBack` [expert]
- [ ] 3.4 Hold the lazy default connection in `HostContextBridge` (`Option<Box<dyn ExaConnection>>`); build params from `UdfMeta` handshake credentials [expert]
- [ ] 3.5 Validate annotated schema against `exascript_metadata` at load; close session with `F-UDF-CL-RUST-####` on mismatch
- [ ] 3.6 Runtime unit tests: single-call dispatch (incl. VS adapter), undefined-call, connect-back query (mock `ExaConnection`), schema-mismatch close

### Group D — cargo-exaudf CLI (independent of A/B/C)

- [ ] 4.1 Implement `cargo exaudf` subcommand dispatch in `cargo-exaudf/src/main.rs`
- [ ] 4.2 Implement `new`: scaffold Cargo.toml (cdylib + SDK dep) + `src/lib.rs` stub; refuse non-empty target
- [ ] 4.3 Implement `build`: `rustup target add` if missing, `cargo build --release --target x86_64-unknown-linux-musl`, print `.so` path, emit `<crate>.udf-meta.json` sidecar for annotated crates
- [ ] 4.4 Implement `validate`: dlopen `.so`, resolve `__exa_udf_entry`, compare `abi_version` + `sdk_fingerprint`, report mismatches
- [ ] 4.5 CLI tests for new/build/validate happy paths and error paths (existing-target, missing-symbol, ABI/fingerprint mismatch)

### Group E — Examples + integration (depends on B, C, D)

- [ ] 5.1 Add `connect-back-query` example UDF (uses `ctx.exa()?.query_arrow`) under the test-udfs set
- [ ] 5.2 Add `annotated-double` example UDF using the typed annotation
- [ ] 5.3 Add connect-back roundtrip integration test in `crates/it/tests/`
- [ ] 5.4 Add single-call `SC_FN_DEFAULT_OUTPUT_COLUMNS` and `SC_FN_GENERATE_SQL_FOR_EXPORT_SPEC` (undefined) roundtrip integration tests
- [ ] 5.5 Add `connect-back-insert` example UDF: for each input row calls `ctx.exa()?.execute(CREATE TABLE IF NOT EXISTS cb_result)` then `ctx.exa()?.execute(INSERT INTO cb_result VALUES (...))`, emits row count; builds for musl target
- [ ] 5.6 Add DML connect-back integration test: invoke UDF over live container, then shell out to `exapump` with `validateservercertificate=0` to `SELECT val FROM cb_result ORDER BY val` and assert the three inserted values `[10, 20, 30]` are present

### Group F — Connect-back root-cause and fix (v2 completion)

- [ ] 6.1 Confirm the credential path: assert via the wire that the handshake carries no `connection_information` and the on-demand `MT_IMPORT` (`PB_IMPORT_CONNECTION_INFORMATION`) path supplies `kind`/`address`/`user`/`password`; record findings in the decision log
- [ ] 6.2 Verify that the native exarrow-rs transport connects to a routable `2026.1.0` endpoint without crashing the session; native is the required transport per design decision (see ADR in `decision-log.md`) [expert]
- [ ] 6.3 Fix `crates/exa-udf-runtime/src/connect_back.rs` to connect to the named-connection `address` as an external client (drop the internal-proxy assumption; remove `transport=websocket` from the DSN — no transport override is needed since the native feature is the exarrow-rs default) [expert]
- [ ] 6.4 Fix the integration harness so `CB_SELF` is created `TO '<routable-endpoint>:8563'` reachable from the UDF sandbox network namespace, replacing the loopback/eth0 address that caused the SIGABRT [expert]
- [ ] 6.5 Remove the `/tmp/cb_debug.txt` `debug_write` instrumentation from `connect_back.rs`
- [ ] 6.6 Update `dsn_disables_cert_validation_and_carries_credentials` unit test to assert the corrected DSN (no `transport=` override so native is used, cert validation disabled)

### Group G — Version bump to 0.2.0 (depends on F)

- [ ] 7.1 Bump `version = "0.2.0"` in every workspace crate `Cargo.toml`: `exaudfclient`, `exa-udf-runtime`, `exasol-udf-sdk`, `exa-zmq-protocol`, `exa-proto`, `exasol-udf-macros`, `cargo-exaudf`, `it`
- [ ] 7.2 Update any inter-crate path-dep `version` requirements to match `0.2.0` and confirm `cargo build --release` still resolves
- [ ] 7.3 Bump `abi_version` / SDK fingerprint references in docs only if the ABI changed in F (it should not; record either way)

### Group H — Git branch and commit (depends on G)

- [ ] 8.1 Create branch `v2` (off the current default branch) and switch to it
- [ ] 8.2 Stage and commit the connect-back fix + version bump with a Conventional Commits message (e.g. `fix(connect-back): connect to named-connection address as external client; bump to 0.2.0`)

### Group I — Verification (depends on H)

- [ ] 9.1 Run `cargo build --release`, `cargo test`, clippy, and `cargo fmt --check` — all clean
- [ ] 9.2 Run the full integration suite against live `2026.1.0`; assert `connect_back_udf_queries_and_emits` returns `42` and the parent session survives [expert]
- [ ] 9.3 Run `connect_back_dml_inserts_visible_via_exapump`; assert `exapump` SELECT returns `[10, 20, 30]` with `validateservercertificate=0`
- [ ] 9.4 Flip V.6 to done in `tasks.md` once 9.2 and 9.3 pass

## Parallelization

| Parallel Group | Tasks |
|----------------|-------|
| Group A | 1.1, 1.2, 1.3, 1.4, 1.5 |
| Group B | 2.1, 2.2, 2.3, 2.4, 2.5 |
| Group D | 4.1, 4.2, 4.3, 4.4, 4.5 |
| Group C | 3.1–3.6 |
| Group E | 5.1–5.6 |
| Group F | 6.1, 6.5, 6.6 (6.2→6.3→6.4 serial) |
| Group G | 7.1, 7.2, 7.3 |

Sequential dependencies:
- A, B, D can run concurrently (D is fully independent of the protocol/SDK work)
- C depends on A (event/action shapes) and B (trait + vtable layout)
- E depends on B + C (examples/integration) and D (build/deploy tooling)
- F (connect-back completion) follows E; within F, 6.2 → 6.3 → 6.4 are serial (diagnose → fix client → fix harness), while 6.1/6.5/6.6 can run alongside
- G (version bump) follows F; H (branch + commit) follows G; I (verification) follows H

## Dead Code Removal

| Type | Location | Reason |
|------|----------|--------|
| Stub | `crates/cargo-exaudf/src/main.rs` (`eprintln!` + `exit(1)`) | Replaced by real subcommand dispatch |
| (none) | `crates/exa-udf-runtime/src/compiler.rs` | Kept as-is returning `UnsupportedFeature` — JIT out of scope |

## Verification

### Scenario Coverage

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| Single-call request surfaces a SingleCall host event | Unit | `crates/exa-zmq-protocol/tests/single_call.rs` | `mt_call_emits_single_call_event` |
| Single-call return is serialized to MT_RETURN | Unit | `crates/exa-zmq-protocol/tests/single_call.rs` | `single_call_return_serializes_mt_return` |
| Unimplemented single-call hook is serialized to MT_UNDEFINED_CALL | Unit | `crates/exa-zmq-protocol/tests/single_call.rs` | `undefined_call_serializes_mt_undefined_call` |
| Connection information is surfaced from the handshake info response | Unit | `crates/exa-zmq-protocol/tests/transport.rs` | `info_event_carries_connection_information` |
| Unexpected message in a phase is a protocol error | Unit | `crates/exa-zmq-protocol/tests/transport.rs` | `mt_call_in_run_phase_is_protocol_error` |
| Single-call mode routes to the single-call dispatcher | Unit | `crates/exa-udf-runtime/tests/single_call.rs` | `single_call_mode_routes_to_dispatcher` |
| Single-call dispatch invokes the matching vtable hook and returns | Unit | `crates/exa-udf-runtime/tests/single_call.rs` | `dispatch_invokes_default_output_columns` |
| Unimplemented single-call hook replies MT_UNDEFINED_CALL | Unit | `crates/exa-udf-runtime/tests/single_call.rs` | `unimplemented_hook_replies_undefined_call` |
| Virtual-schema adapter call is dispatched to the adapter hook | Unit | `crates/exa-udf-runtime/tests/single_call.rs` | `dispatch_invokes_virtual_schema_adapter_call` |
| Connect-back opens a connection from the handshake credentials | Unit | `crates/exa-udf-runtime/tests/connect_back.rs` | `exa_opens_and_caches_connection` |
| Connect-back query returns Arrow batches to the UDF | Unit | `crates/exa-udf-runtime/tests/connect_back.rs` | `query_arrow_returns_record_batches` |
| Annotated schema is validated against the database metadata at load | Unit | `crates/exa-udf-runtime/tests/dispatch.rs` | `annotated_schema_mismatch_closes_session` |
| ExaConnection trait is defined behind the connect-back feature | Unit | `crates/exasol-udf-sdk/tests/connect_back.rs` | `exaconnection_trait_surface` |
| UdfContext connect-back methods are absent without the feature | Unit | `crates/exasol-udf-sdk/tests/feature_gate.rs` | `connect_back_methods_absent_without_feature` |
| UdfContext exposes connect-back methods with the feature | Unit | `crates/exasol-udf-sdk/tests/connect_back.rs` | `udfcontext_exposes_exa_methods` |
| UdfRun default single-call hooks return Unimplemented | Unit | `crates/exasol-udf-sdk/tests/udf_run.rs` | `default_single_call_hooks_unimplemented` |
| ABI constants and vtable layout are stable | Unit | `crates/exasol-udf-sdk/tests/abi.rs` | `vtable_layout_includes_vs_adapter` |
| exasol_udf annotation generates schema metadata for matching types | Unit | `crates/exasol-udf-macros/tests/annotation.rs` | `annotation_maps_types_to_exatype` |
| exasol_udf annotation with an unknown type fails to compile | Unit (trybuild) | `crates/exasol-udf-macros/tests/trybuild/bad_annotation_type.rs` | `trybuild::annotation_unknown_type` |
| new scaffolds a buildable UDF crate | Integration | `crates/cargo-exaudf/tests/new.rs` | `new_scaffolds_buildable_crate` |
| new rejects an existing non-empty target | Integration | `crates/cargo-exaudf/tests/new.rs` | `new_rejects_existing_target` |
| build produces a fully-static musl .so | Integration | `crates/cargo-exaudf/tests/build.rs` | `build_produces_musl_so` |
| build installs the musl target when missing | Integration | `crates/cargo-exaudf/tests/build.rs` | `build_installs_missing_target` |
| build emits a schema sidecar for annotated UDFs | Integration | `crates/cargo-exaudf/tests/build.rs` | `build_emits_schema_sidecar` |
| validate accepts a compatible .so | Integration | `crates/cargo-exaudf/tests/validate.rs` | `validate_accepts_compatible_so` |
| validate rejects an ABI or fingerprint mismatch | Integration | `crates/cargo-exaudf/tests/validate.rs` | `validate_rejects_abi_or_fingerprint_mismatch` |
| validate rejects a .so missing the entry symbol | Integration | `crates/cargo-exaudf/tests/validate.rs` | `validate_rejects_missing_entry` |
| connect-back-query emits a value fetched over connect-back | Unit | `crates/exa-udf-runtime/tests/examples.rs` | `connect_back_query_example_builds_and_emits` |
| annotated-double declares its schema via the typed annotation | Unit | `crates/exasol-udf-macros/tests/annotation.rs` | `annotated_double_embeds_schema` |
| connect-back-insert creates a table and writes rows during run | Unit | `crates/exa-udf-runtime/tests/examples.rs` | `connect_back_insert_example_builds` |
| Connect-back UDF queries the database and emits the result | Integration | `crates/it/tests/db_roundtrip.rs` | `connect_back_udf_queries_and_emits` |
| Connect-back DML UDF inserts rows and data is visible externally | Integration | `crates/it/tests/db_roundtrip.rs` | `connect_back_dml_inserts_visible_via_exapump` |
| Single-call default-output-columns roundtrip returns a schema | Integration | `crates/it/tests/db_roundtrip.rs` | `single_call_default_output_columns_roundtrip` |
| Unimplemented single-call hook surfaces an undefined-call response | Integration | `crates/it/tests/db_roundtrip.rs` | `single_call_unimplemented_returns_undefined` |
| Connect-back retrieves credentials on demand when the handshake carries none | Unit | `crates/exa-udf-runtime/tests/connect_back.rs` | `on_demand_mt_import_supplies_credentials` |
| Connect-back connects to the named connection address like an external client | Unit | `crates/exa-udf-runtime/src/connect_back.rs` | `dsn_disables_cert_validation_and_carries_credentials` |
| Connect-back UDF reaches a routable database endpoint without crashing the session | Integration | `crates/it/tests/db_roundtrip.rs` | `connect_back_udf_queries_and_emits` |

### Manual Testing

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| tools/cargo-exaudf | `cargo exaudf new /tmp/my-udf && (cd /tmp/my-udf && cargo build)` | Crate scaffolded; builds clean |
| tools/cargo-exaudf | `cd /tmp/my-udf && cargo exaudf build` | Prints `target/x86_64-unknown-linux-musl/release/libmy_udf.so` |
| tools/cargo-exaudf | `cargo exaudf validate /tmp/my-udf/target/x86_64-unknown-linux-musl/release/libmy_udf.so` | Exit 0, reports ABI + fingerprint match |
| sdk/udf-sdk (annotation) | `cargo build -p exasol-udf-macros` then build the `annotated-double` example | Vtable embeds `x: Int64` / `result: Int64` |
| sdk/udf-sdk (connect-back) | `cargo build -p exasol-udf-sdk --features connect-back` | Compiles with `ExaConnection` + `exa()` present |
| runtime/host-dispatch | `cargo test -p exa-udf-runtime single_call` | Single-call + VS adapter dispatch tests pass |
| integration/db-roundtrip | `cargo test -p it connect_back_udf_queries_and_emits` (Exasol `2026.latest` running) | Connect-back UDF emits `42` |

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Build | `cargo build --release` | Exit 0 |
| Test | `cargo test` | 0 failures |
| Lint | `cargo clippy --all-targets --all-features -- -D warnings` | 0 warnings |
| Format | `cargo fmt --check` | No changes |
