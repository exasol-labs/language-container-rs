# Verification Report: add-udf-cleanup-hook

## Verdict

| Result | Details |
|--------|---------|
| **PASS** | The `cleanup(path)` hook lands at ABI 11. All 57 build/test/lint/format/integration checks are green, including the five new live scenarios. |
| Code review | 9 findings — 9 fixed |

| Check | Status |
|-------|--------|
| Build | ✓ |
| Tests | ✓ |
| Lint | ✓ |
| Format | ✓ |
| Scenario Coverage | ✓ |
| Manual Tests | ✓ |

## Test Evidence

### Coverage

| Type | Coverage % |
|------|------------|
| Unit | Not measured (`cargo llvm-cov` not run this pass) |
| Integration | Not measured |

### Test Results

| Type | Run | Passed | Ignored |
|------|-----|--------|---------|
| Unit + Integration (`cargo test`, full workspace) | 424 | 424 | 0 |
| Runtime, all features (`cargo test -p exa-udf-runtime --all-features`) | 209 | 209 | 0 |
| Live DB integration (`cargo test -p it --features integration`) | 57 scenarios (1 test binary) | 57 | 0 |

### Manual Tests

| Test | Result |
|------|--------|
| `cargo test -p exasol-udf-sdk abi` | ✓ `cleanup_slot_takes_context_and_abi_version_is_eleven` passes |
| `cargo test -p exasol-udf-macros` | ✓ the five `cleanup` tests + trybuild case pass |
| `cargo test -p exa-udf-runtime --all-features --test dispatch cleanup` | ✓ 0 failures, includes `cleanup_connection_lookup_is_refused_without_mt_import` |
| `cargo test -p exa-udf-runtime cleanup` (no connect-back feature) | ✓ 0 failures |
| `cargo test -p exa-udf-runtime --all-features --test single_call cleanup` | ✓ 0 failures |
| `cargo build --release -p cleanup-hook && cargo exasol-udf validate target/release/libcleanup_hook.so` | ✓ 6 entry points validate at ABI 11, SDK fingerprint 0.30.0 |
| `cargo test -p it --features integration` | ✓ all five `cleanup_*`/`export_into_script_fails_on_cleanup_error` scenarios print `[it] scenario <name> ok` on stderr |

## Tool Evidence

### Linter

```
cargo clippy --all-targets --all-features -- -D warnings
Finished `dev` profile [unoptimized + debuginfo] target(s) in 3.92s
(exit 0, 0 warnings)
```

### Formatter

```
cargo fmt --check
(exit 0, no changes)
```

## Scenario Coverage

| Domain | Feature | Scenario | Test Location | Test Name | Passes |
|--------|---------|----------|---------------|-----------|--------|
| sdk | udf-abi | cleanup annotation wires the cleanup slot | `crates/exasol-udf-macros/tests/cleanup.rs` | `cleanup_annotation_wires_slot_and_maps_outcomes` | Pass |
| sdk | udf-abi | cleanup absent leaves the slot None | `crates/exasol-udf-macros/tests/cleanup.rs` | `omitted_cleanup_leaves_slot_none` | Pass |
| sdk | udf-abi | cleanup slot replaces destroy, ABI bumps to 11 | `crates/exasol-udf-sdk/src/abi_tests.rs` | `cleanup_slot_takes_context_and_abi_version_is_eleven` | Pass |
| sdk | udf-macro | macro generates entry point + vtable | `crates/exasol-udf-macros/tests/cleanup.rs` | `cleanup_shim_carries_the_entry_suffix` | Pass |
| sdk | udf-macro | function name → UPPER_SNAKE_CASE SQL name | `crates/exasol-udf-macros/tests/cleanup.rs` | `cleanup_shim_carries_the_entry_suffix` | Pass |
| sdk | udf-macro | name attribute overrides SQL entry point name | `crates/exasol-udf-macros/tests/cleanup.rs` | `name_combines_with_cleanup_section` | Pass |
| sdk | udf-macro | distinct exasol_udf annotations get independent entry points | `crates/exasol-udf-macros/tests/cleanup.rs` | `distinct_entries_get_independent_cleanup_shims` | Pass |
| sdk | udf-macro | unknown annotation section rejected | `crates/exasol-udf-macros/tests/trybuild/unknown_annotation_section.rs` | (trybuild) | Pass |
| runtime | dispatch-run-loop | cleanup runs once after the last group, before MT_FINISHED | `crates/exa-udf-runtime/tests/dispatch.rs` | `cleanup_runs_once_after_the_last_group`, `successful_cleanup_precedes_mt_finished`, `cleanup_runs_when_no_group_ran` | Pass |
| runtime | dispatch-run-loop | same, live DB | `crates/it/tests/db_roundtrip.rs` | `cleanup_ok_statement_succeeds`, `cleanup_reports_per_process_counts` | Pass |
| runtime | dispatch-run-loop | cleanup error fails the statement instead of MT_FINISHED | `crates/exa-udf-runtime/tests/dispatch.rs`, `crates/it/tests/db_roundtrip.rs` | `cleanup_runs_once_after_the_last_group`, `cleanup_reports_per_process_counts` | Pass |
| runtime | dispatch-run-loop | error that ends dispatch still runs cleanup, reports both errors | `crates/exa-udf-runtime/tests/dispatch.rs`, `src/cleanup_tests.rs`, `crates/it/tests/db_roundtrip.rs` | `run_error_runs_cleanup_and_reports_both_errors`, `db_close_runs_cleanup_before_relaying_the_close`, `fold_keeps_the_original_error_first`, `run_and_cleanup_errors_both_surface` | Pass |
| runtime | dispatch-run-loop | validation failure skips cleanup | `crates/exa-udf-runtime/tests/dispatch.rs` | `output_shape_mismatch_skips_cleanup`, `schema_mismatch_skips_cleanup` | Pass |
| runtime | dispatch-run-loop | CleanupContext carries handshake metadata + connect-back | `src/rowset_tests.rs`, `tests/dispatch.rs`, `crates/it/tests/db_roundtrip.rs` | `cleanup_context_refuses_connection_lookup`, `cleanup_runs_once_after_the_last_group`, `cleanup_reports_per_process_counts`, `cleanup_connects_back_with_a_resolved_connection_object` | Pass |
| runtime | dispatch-run-loop | CleanupContext refuses CONNECTION lookups without MT_IMPORT | `src/rowset_tests.rs`, `tests/dispatch.rs`, `tests/single_call.rs`, `crates/it/tests/db_roundtrip.rs` | `cleanup_context_refuses_connection_lookup`, `cleanup_connection_lookup_is_refused_without_mt_import`, `single_call_cleanup_runs_before_finished`, `cleanup_connects_back_with_a_resolved_connection_object` | Pass |
| runtime | dispatch-single-call | cleanup hook runs after the single-call loop | `crates/exa-udf-runtime/tests/single_call.rs`, `crates/it/tests/db_roundtrip.rs` | `single_call_cleanup_runs_before_finished`, `single_call_error_still_runs_cleanup`, `export_into_script_fails_on_cleanup_error` | Pass |

## Notes

- Code review found 9 findings (8 standard, 1 expert); all fixed. The expert fix (unifying the "append recorded detail" logic between `cleanup.rs` and `single_call.rs` into `RuntimeError::with_recorded_detail`) changed one visible error string on the single-call adapter path: a doubled `UDF error: UDF error:` prefix became a single prefix. Covered by an added assertion in `adapter_connection_probe_combines_hook_and_recorded_errors`.
- Coverage percentages were not measured this pass (`cargo llvm-cov` not run); all Scenario Coverage rows above are traced to specific named tests that ran green, per the table.
- `scripts/ci-it-local.sh`'s fixture `-p` allowlist is unchanged and was already missing several fixtures before this plan (including now `cleanup-hook`); the actual `cargo test -p it --features integration` run in this verification pass built the fixture directly and does not depend on that script. Out of scope for this plan per Group B's implementer note.
- Full evidence logs: `target/speq-build.log`, `target/speq-test.log`, `target/speq-test-runtime-allfeatures.log`, `target/speq-clippy.log`, `target/speq-fmt.log`, `target/speq-integration.log`, `target/speq-fixtures-rebuild.log` (all local to the working tree, not committed).
