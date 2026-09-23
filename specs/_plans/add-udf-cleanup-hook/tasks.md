# Tasks: add-udf-cleanup-hook

## PR Lifecycle
- [x] resolved
- [x] implemented
- [x] version-bumped
- [ ] tested-green
- [ ] recorded
- [ ] pr-ready

## Phase 2: Implementation (Group A: Cleanup slot and macro)
- [x] 1.1 Replace `destroy` with `cleanup: Option<...>` in abi.rs, bump `EXA_UDF_ABI_VERSION` to 11, update abi_tests.rs, add `cleanup_slot_takes_context_and_abi_version_is_eleven`
- [x] 1.2 Parse `cleanup(path)` section into `Annotations`, add `cleanup` to unknown-section error + trybuild stderr
- [x] 1.3 Extract shared `(ctx, error_out) -> i32` shim builder, generate `__exa_cleanup_shim_<NAME>` when present, delete unconditional destroy shim [expert]
- [x] 1.4 Add `crates/exasol-udf-macros/tests/cleanup.rs` with the five macro tests
- [x] 1.5 Rename `destroy` to `cleanup` in hand-written vtables (single-call-fixture, cargo-exasol-udf validate.rs + tests)

## Phase 2: Implementation (Group B: Session teardown)
- [x] 2.1 Replace `LoadedUdf::destroy` with `LoadedUdf::cleanup`, shared loader helper for return-code/out-pointer handling, update loader_tests.rs + tests/loader.rs
- [x] 2.2 Add `CleanupContext` in rowset.rs, split delegation macros, refuse `connection`, add `crates/exa-udf-runtime/src/cleanup.rs` + cleanup_tests.rs [expert]
- [x] 2.3 Remove trailing `finished_reply` exchange from both dispatchers, `Runtime::run` owns teardown, delete pre-dispatch destroy calls, update doc comments [expert]
- [x] 2.4 Add `test-udfs/cleanup-hook` fixture crate, wire into workspace members/dev-dependencies/CI allowlist
- [x] 2.5 Add run-loop mock tests to `crates/exa-udf-runtime/tests/dispatch.rs`
- [x] 2.6 Add `single_call_cleanup_runs_before_finished` and `single_call_error_still_runs_cleanup` to tests/single_call.rs
- [x] 2.7 Upload `libcleanup_hook.so` in `crates/it/tests/db_roundtrip.rs`, add five live scenarios
- [x] 2.8 Document cleanup(path) in docs/writing-a-udf.md, docs/protocol.md, specs/architecture.md

## Phase 2: Implementation (Group C: Release hygiene)
- [x] 3.1 Bump workspace version to 0.30.0, update exasol-udf-sdk pin, regenerate Cargo.lock, rebuild test-udfs/*.so

## Phase 3: Verification
- [x] 3.2 Build: `cargo build --release`
- [x] 3.3 Test: `cargo test`
- [x] 3.4 Test, all features: `cargo test -p exa-udf-runtime --all-features`
- [x] 3.5 Integration: `cargo test -p it --features integration`
- [x] 3.6 Lint: `cargo clippy --all-targets --all-features -- -D warnings`
- [x] 3.7 Format: `cargo fmt --check`

## Phase 4: Review Fixes
- [x] 4.1 In crates/exa-udf-runtime/src/cleanup.rs, make `invoke_hook` return `Result<(), RuntimeError>` and change `fold`'s second parameter to `cleanup: Result<(), RuntimeError>` with a three-arm match; update cleanup_tests.rs `fold` calls to pass `Ok(())`/`Err(..)`
- [x] 4.2 In crates/exasol-udf-macros/src/lib.rs, add private `parse_path_section(input: ParseStream) -> syn::Result<Path>` next to `parse_schema_fields` and use it in the `vs_adapter`, `import_spec`, `export_spec`, and `cleanup` arms
- [x] 4.3 In crates/exa-udf-runtime/tests/dispatch.rs, replace the inlined close-receive block in `cleanup_connection_lookup_is_refused_without_mt_import` with `expect_close(&server, ..)`
- [x] 4.4 In crates/exasol-udf-sdk/src/abi_tests.rs, delete the two comment lines above `fail_with_ctx_presence` in `cleanup_slot_takes_context_and_abi_version_is_eleven`
- [x] 4.5 In crates/it/tests/db_roundtrip.rs, delete the `// Cleanup-hook scenarios.` line in `db_roundtrip_all_scenarios`
- [x] 4.6 In docs/writing-a-udf.md, fix the cleanup section's timing (line 185), validation-failure (line 189), and panic (line 193) prose
- [x] 4.7 In docs/protocol.md, name the mid-group `MT_NEXT` answered with `MT_CLEANUP` in the "3. Cleanup." step
- [x] 4.8 In specs/architecture.md, replace the `cleanup` clause of the **Lifecycle:** bullet with the dispatch-end wording (MT_FINISHED + `exit(0)`, or error MT_CLOSE + non-zero exit)
- [x] 4.9 Move `with_recorded_detail` to `RuntimeError::with_recorded_detail` in crates/exa-udf-runtime/src/error.rs, call it from cleanup.rs `invoke_hook` and single_call.rs `invoke_ctx_hook`, and move its three tests into a new error_tests.rs [expert]
