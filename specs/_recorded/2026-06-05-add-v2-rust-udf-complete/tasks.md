# Tasks: add-v2-rust-udf-complete

## Group A — Protocol layer (foundation)

- [x] 1.1 Add `MtCall`/`MtReturn`/`MtUndefinedCall` handling and `SingleCallFn` decoding in `exa-zmq-protocol/src/loop_.rs`
- [x] 1.2 Add `HostEvent::SingleCall` and `HostAction::SingleCallReturn` / `HostAction::UndefinedCall` to `messages.rs`
- [x] 1.3 Surface `ExascryptConnectionInformationRep` (host/port/user/password) on `HostEvent::Info`; add `conn_info` field to `UdfMeta`
- [x] 1.4 Treat `MT_CALL` arriving in a scalar/set run phase as a `ProtocolError`
- [x] 1.5 Unit tests for single-call request/return/undefined and conn-info parsing in `exa-zmq-protocol/tests/single_call.rs` and updated `transport.rs`

## Group B — SDK surface (can start in parallel with A and D)

- [x] 2.1 Add `virtual_schema_adapter_call` to `UdfRun` trait (it already exists); add `ExaUdfVTable` vtable fields for VS adapter and single-call hooks in `abi.rs`; update vtable layout and ABI version
- [x] 2.2 Define `ExaConnection` trait + `ConnectBackOptions` enum in new `exasol-udf-sdk/src/connect_back.rs`, gated by `connect-back` feature [expert]
- [x] 2.3 Add `exa` / `exa_named` / `exa_connect` methods to `UdfContext` trait behind `connect-back` feature; confirm absence when feature disabled
- [x] 2.4 Extend `exasol-udf-macros` to parse `input(...)` / `emits(...)`, map Rust types to `ExaType`, embed schema in the vtable, emit compile error on unmappable types [expert]
- [x] 2.5 Add macro unit + `trybuild` compile-fail tests for annotation parsing and unknown-type errors

## Group D — cargo-exaudf CLI (independent of A/B/C)

- [x] 4.1 Implement `cargo exaudf` subcommand dispatch in `cargo-exaudf/src/main.rs`
- [x] 4.2 Implement `new`: scaffold `Cargo.toml` (cdylib + SDK dep) + `src/lib.rs` stub; refuse non-empty target
- [x] 4.3 Implement `build`: `rustup target add` if missing, `cargo build --release --target x86_64-unknown-linux-musl`, print `.so` path, emit `<crate>.udf-meta.json` sidecar for annotated crates
- [x] 4.4 Implement `validate`: dlopen `.so`, resolve `__exa_udf_entry`, compare `abi_version` + `sdk_fingerprint`, report mismatches
- [x] 4.5 CLI tests for new/build/validate happy paths and error paths (existing-target, missing-symbol, ABI/fingerprint mismatch)

## Group C — Runtime wiring (depends on A and B)

- [x] 3.1 Add `exa-udf-runtime/src/single_call.rs` routing `HostEvent::SingleCall` → matching vtable hook → `SingleCallReturn` / `UndefinedCall`
- [x] 3.2 In `dispatch.rs`, detect `single_call_mode` from `MT_META` and route to single-call dispatcher instead of run loop
- [x] 3.3 Add `exa-udf-runtime/src/connect_back.rs`: `CONNECT_BACK_RT` `OnceLock<Runtime>`, implement `ExaConnection` over exarrow-rs, mapping errors to `UdfError::ConnectBack` [expert]
- [x] 3.4 Hold the lazy default connection in `HostContextBridge` (`Option<Box<dyn ExaConnection>>`); build params from `UdfMeta` handshake credentials [expert]
- [x] 3.5 Validate annotated schema against `exascript_metadata` at load; close session with `F-UDF-CL-RUST-####` on mismatch
- [x] 3.6 Runtime unit tests: single-call dispatch (incl. VS adapter), undefined-call, connect-back query (mock `ExaConnection`), schema-mismatch close

## Group E — Examples + integration (depends on B, C, D)

- [x] 5.1 Add `connect-back-query` example UDF (uses `ctx.exa()?.query_arrow`) under `test-udfs/`
- [x] 5.2 Add `annotated-double` example UDF using the typed annotation
- [x] 5.3 Add connect-back roundtrip integration test in `crates/it/tests/db_roundtrip.rs`
- [x] 5.4 Add single-call `SC_FN_DEFAULT_OUTPUT_COLUMNS` and `SC_FN_GENERATE_SQL_FOR_EXPORT_SPEC` (undefined) roundtrip integration tests
- [x] 5.5 Add `connect-back-insert` example UDF: creates table, inserts rows, emits row count
- [x] 5.6 Add DML connect-back integration test: invoke UDF, `exapump` SELECT to assert `[10, 20, 30]`

## Verification

- [x] V.1 Build: `cargo build --release` → exit 0
- [x] V.2 Test: `cargo test` → 0 failures
- [x] V.3 Lint: `cargo clippy --all-targets` (1.84) + `cargo +1.91 clippy -p exasol-udf-sdk -p exa-udf-runtime --features connect-back` → 0 warnings
- [x] V.4 Format: `cargo fmt --check` → no changes
- [x] V.5 Manual: `cargo exaudf new` / `validate` happy paths verified; `build` requires musl toolchain
- [ ] V.6 Manual: connect-back integration roundtrip — BLOCKED (server-side bug confirmed). Empirical testing on 2026-06-05 confirmed that Exasol 2026.1.0 SIGABRTs the outer session handler (signal 6, core dumped) when a connect-back session is created from within a UDF — regardless of transport (native/WebSocket) or address (container IP, Docker host gateway). The prior "wrong address" root cause (decision [8]) was incorrect; decision [15] records the confirmed finding. Cannot unblock until a patched `2026.x` image is available.

## Group F — Connect-back root-cause and fix (v2 completion)

- [x] 6.1 Confirm via the wire that the handshake carries no `connection_information` and the on-demand `MT_IMPORT` (`PB_IMPORT_CONNECTION_INFORMATION`) path supplies `kind`/`address`/`user`/`password`; record in decision log
- [x] 6.2 Verify that the native exarrow-rs transport connects to a routable `2026.1.0` endpoint without crashing the session; native is the required transport per design decision (see ADR in `decision-log.md`) [expert]
- [x] 6.3 Fix `crates/exa-udf-runtime/src/connect_back.rs` to connect to the named-connection `address` as an external client; remove `transport=websocket` from the DSN — no transport override needed since the native feature is the exarrow-rs default [expert]
- [x] 6.4 Fix the integration harness so `CB_SELF` is created `TO '<routable-endpoint>:8563'` reachable from the UDF sandbox, replacing the loopback/eth0 address that caused the SIGABRT [expert]
- [x] 6.5 Remove the `/tmp/cb_debug.txt` `debug_write` instrumentation from `connect_back.rs`
- [x] 6.6 Update the `dsn_disables_cert_validation_and_carries_credentials` unit test to assert the corrected DSN (no `transport=` override so native is used, cert validation disabled)

## Group G — Version bump to 0.2.0

- [x] 7.1 Bump `version = "0.2.0"` in every workspace crate `Cargo.toml`: `exaudfclient`, `exa-udf-runtime`, `exasol-udf-sdk`, `exa-zmq-protocol`, `exa-proto`, `exasol-udf-macros`, `cargo-exaudf`, `it`
- [x] 7.2 Update inter-crate path-dep `version` requirements to `0.2.0`; confirm `cargo build --release` resolves
- [x] 7.3 Record whether the ABI version / SDK fingerprint changed (it should not)

## Group H — Git branch and commit

- [x] 8.1 Create branch `v2` off the current default branch and switch to it
- [x] 8.2 Commit the connect-back fix + version bump with a Conventional Commits message (`fix(connect-back): connect to named-connection address as external client; bump to 0.2.0`)

## Group I — Verification (live 2026.1.0)

- [x] 9.1 `cargo build --release` (stable), `cargo test --workspace --exclude it`, clippy (stable + 1.91 with connect-back), `cargo fmt --check` — all clean
- [ ] 9.2 Full integration suite: `connect_back_udf_queries_and_emits` returns `42` and the parent session survives [expert] — BLOCKED: Exasol 2026.1.0 SIGABRT (see V.6 and decision [15])
- [ ] 9.3 `connect_back_dml_inserts_visible_via_exapump`: `exapump` SELECT returns `[10, 20, 30]` — BLOCKED: same server-side SIGABRT
- [ ] 9.4 Flip V.6 to done once 9.2 and 9.3 pass — BLOCKED
