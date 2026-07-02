# Tasks: fix-single-call-handshake-metadata

## Phase 2: Implementation (Group A)
- [x] 2.1 Add `handshake: HandshakeMeta` field to `SingleCallContext<'a>` and thread through `SingleCallContext::new(...)` (crates/exa-udf-runtime/src/rowset.rs)

## Phase 2: Implementation (Group B)
- [x] 2.2 Add 13 handshake accessor overrides to `impl UdfContext for SingleCallContext<'_>`, mirroring HostContextBridge (crates/exa-udf-runtime/src/rowset.rs) [expert]
- [x] 2.3 Thread handshake through single-call dispatcher: rename `_meta`→`meta`, build `HandshakeMeta::from(meta)`, thread through invoke_hook/invoke_vs_adapter_call/SingleCallContext::new (crates/exa-udf-runtime/src/single_call.rs) [expert]
- [x] 2.4 Fix two existing in-crate `SingleCallContext::new(...)` unit-test call sites to pass `HandshakeMeta::default()` (crates/exa-udf-runtime/src/rowset.rs)

## Phase 2: Implementation (Group C)
- [x] 2.5 Add `single_call_context_returns_handshake_metadata` unit test mirroring `bridge_returns_handshake_metadata` (crates/exa-udf-runtime/src/rowset.rs test module)
- [x] 2.6 Extend `single-call-fixture` adapter shim to read live metadata off ctx pointer and surface via rc=1 error string (test-udfs/single-call-fixture/src/lib.rs) [expert]
- [x] 2.9 Update `crates/exa-udf-runtime/tests/single_call.rs` for the new rc=1 adapter contract: rewrite `dispatch_invokes_virtual_schema_adapter_call` → `dispatch_surfaces_adapter_hook_error` (asserts MT_CLOSE + HANDSHAKE_META error text) and switch `mt_return_ack_terminates_session` to the still-succeeding `default_output_columns` hook [expert]

## Phase 2: Implementation (Group D)
- [x] 2.7 Add `single_call_adapter_surfaces_live_handshake_metadata` IT scenario + wire into `db_roundtrip_all_scenarios` (crates/it/tests/db_roundtrip.rs) [expert]

## Phase 2: Implementation (Group E)
- [x] 2.8 Bump version 0.20.0 → 0.20.1 (workspace.package + exasol-udf-sdk pin) and regenerate Cargo.lock

## Phase 2: Implementation (Group C follow-up)
- [x] 2.9 Update pre-existing `dispatch_invokes_virtual_schema_adapter_call` in crates/exa-udf-runtime/tests/single_call.rs to match new rc=1 HANDSHAKE_META fixture contract (was `{"echo":{}}`) [expert]

## Phase 4: Code Review
- [x] 4.1 Review all changed files — clean, no blocking findings (2 optional low-sev observations, not actioned per reviewer)

## Phase 5: Verification
- [x] 5.1 Build (cargo build --release) — exit 0
- [x] 5.2 Unit tests (cargo test) — 0 failures; new `single_call_context_returns_handshake_metadata` + rewritten `dispatch_surfaces_adapter_hook_error` pass
- [x] 5.3 Lint (cargo clippy --all-targets --all-features -- -D warnings) — clean
- [x] 5.4 Format (cargo fmt --check) — clean
- [x] 5.5 Integration tests (cargo test -p it --features integration, db-2026-1 / 2026.1.0) — db_roundtrip_all_scenarios ok; new `single_call_adapter_handshake_metadata` scenario passed (live node_count != 0 through the VS-adapter path, confirming issue #41 fix + the DB-populates-adapter-handshake gap)
