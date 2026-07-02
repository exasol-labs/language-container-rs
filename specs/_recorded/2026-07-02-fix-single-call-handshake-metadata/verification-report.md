# Verification Report: fix-single-call-handshake-metadata

## Bottom Line

**PASS.** Issue #41 is fixed and verified end-to-end. The single-call virtual-schema
adapter path now surfaces live `exascript_info` handshake metadata (notably a non-zero
`node_count`) instead of the trait's neutral defaults. All automated gates are green,
including the live-DB integration scenario that also closes the issue's "not yet
verified" gap (that the DB actually populates handshake metadata for the VS-adapter
single-call path).

## Automated Checks

| Step | Command | Result |
|------|---------|--------|
| Build | `cargo build --release` | ✅ exit 0 |
| Unit tests | `cargo test` | ✅ 0 failures |
| Lint | `cargo clippy --all-targets --all-features -- -D warnings` | ✅ clean |
| Format | `cargo fmt --check` | ✅ clean |
| Integration | `cargo test -p it --features integration` (db-2026-1 / docker-db 2026.1.0) | ✅ `db_roundtrip_all_scenarios ... ok`, 0 failures |

## Scenario Coverage

| Scenario | Test Type | Location | Test | Result |
|----------|-----------|----------|------|--------|
| Adapter single-call context surfaces live handshake metadata | Unit | `crates/exa-udf-runtime/src/rowset.rs` | `single_call_context_returns_handshake_metadata` | ✅ pass (default + all-features) |
| Adapter single-call dispatch surfaces the hook's live-metadata error over the wire | Unit | `crates/exa-udf-runtime/tests/single_call.rs` | `dispatch_surfaces_adapter_hook_error` | ✅ pass |
| Adapter single-call context surfaces live handshake metadata (end-to-end live values) | Integration | `crates/it/tests/db_roundtrip.rs` | `single_call_adapter_surfaces_live_handshake_metadata` | ✅ pass (live `node_count != 0`) |

## Notes / Deviations from Plan

- **IT scenario adapter script name.** The plan sketched registering the adapter as
  `sc_adapter`. During verification the DB rejected that with `no entry point found for
  script 'SC_ADAPTER'`: the loader resolves the vtable entry point from the (uppercased)
  script name, and the fixture exports only `__exa_udf_entry_SINGLE_CALL_UDF`. Fixed by
  registering the adapter script as `single_call_udf` (dropping the earlier scalar script
  of that name first) and asserting the surfaced script name is `SINGLE_CALL_UDF`. This is
  a test-harness correction, not a behavior change.
- **Collateral test fix (task 2.9).** The pre-existing `dispatch_invokes_virtual_schema_adapter_call`
  asserted the old rc=0 `{"echo":{}}` fixture behavior. Since the fixture now deliberately
  fails (rc=1) to surface metadata, it was rewritten to `dispatch_surfaces_adapter_hook_error`
  (asserts the MT_CLOSE `F-UDF-CL-RUST-9001` error carrying the `HANDSHAKE_META …` text), and
  `mt_return_ack_terminates_session` was switched to the still-succeeding
  `default_output_columns` hook.
- **Fixture rebuild after version bump.** The 0.20.0 → 0.20.1 bump changes the baked SDK
  fingerprint (`SDK_VERSION:RUSTC_HASH`). All `test-udfs/*` release `.so`s must be rebuilt or
  the loader rejects them with a fingerprint mismatch; `libconnect_back_scalar.so` was stale
  and rebuilt. All 18 scenario-uploaded fixtures verified at `0.20.1` before the green run.
- **E2E.** The repo has no separate E2E suite; the `cargo test -p it --features integration`
  matrix is the end-to-end gate. Validated locally against docker-db `2026.1.0` (the default
  series); CI runs the full `2025.1.11 / 2025.2.1 / 2026.1.0` matrix on the PR.

## Code Review

Clean — no blocking findings. The 13 accessor overrides are an exact parity copy of
`HostContextBridge`; the fixture's `unsafe` double-indirection restore matches the runtime's
`ctx_ptr` construction; the version bump is complete and consistent. Two low-severity,
explicitly-non-recommended observations (a negligible per-hook `HandshakeMeta` clone; a
pre-existing arg-count style) were left as-is.
