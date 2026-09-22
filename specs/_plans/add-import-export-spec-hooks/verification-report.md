# Verification Report: add-import-export-spec-hooks

## Verdict

| Result | Details |
|--------|---------|
| **PASS** | All checklist steps green. Both `crates/it/tests/db_roundtrip.rs` live-DB scenarios this plan added (`import_from_script_roundtrip`, `export_into_script_surfaces_spec`) and the new `rows_in_group_reports_live_group_size` scenario ran against a live Exasol Docker DB and passed after two test-expectation/design corrections surfaced by that live run (see Notes). |
| Code review | 18 findings — 18 fixed (14 standard, 4 expert) |

| Check | Status |
|-------|--------|
| Build | ✓ (`cargo build --release`, exit 0) |
| Tests | ✓ (`cargo test`, 396 passed, 0 failed, 62 suites) |
| Integration | ✓ (`cargo test -p it --features integration`, 6 passed, 0 failed, against a live Exasol Docker DB) |
| Lint | ✓ (`cargo clippy --all-targets --all-features -- -D warnings`, 0 warnings) |
| Format | ✓ (`cargo fmt --check`, no diff) |
| Scenario Coverage | ✓ (25/25 scenarios in plan.md's table have a passing test; 1 renamed/split per review finding 4.16, see Notes) |
| Manual Tests | ✓ (9/9 commands in plan.md's Manual Testing table, see below) |

## Test Evidence

### Test Results

| Type | Run | Passed | Ignored |
|------|-----|--------|---------|
| Unit | 396 | 396 | 0 |
| Integration | 6 (1 live-DB scenario driver covering 25 sub-scenarios + 5 harness unit tests) | 6 | 0 |

### Manual Tests

| Test | Result |
|------|--------|
| `cargo test -p exasol-udf-sdk --all-features` | ✓ |
| `cargo tree -p exasol-udf-sdk --edges normal` (no `serde`/`serde_json`) | ✓ |
| `cargo build -p exasol-udf-sdk --features import` then `--features export` | ✓ (both exit 0) |
| `cargo test -p exasol-udf-sdk --all-features abi` (`EXA_UDF_ABI_VERSION` == 10) | ✓ (4 passed) |
| `cargo test -p exasol-udf-macros` (trybuild case) | ✓ (1 passed) |
| `cargo test -p exa-zmq-protocol` | ✓ |
| `cargo test -p exa-udf-runtime --all-features single_call` | ✓ (13 passed across lib + integration binary) |
| `cargo test -p exa-udf-runtime --all-features rowset` | ✓ |
| `cargo build --release -p import-export-spec -p rows-in-group` | ✓ (`libimport_export_spec.so`, `librows_in_group.so` produced) |
| End-to-end IMPORT/EXPORT/rows-in-group (`cargo test -p it --features integration`) | ✓ (`[it] scenario import_from_script ok`, `[it] scenario export_into_script ok`, `[it] scenario rows_in_group ok` observed on a run with captured stdout) |

## Tool Evidence

### Linter

```
cargo clippy --all-targets --all-features -- -D warnings
Finished, 0 warnings
```

### Formatter

```
cargo fmt --check
(no output — clean)
```

## Scenario Coverage

| Domain | Feature | Scenario | Test Location | Test Name | Passes |
|--------|---------|----------|---------------|-----------|--------|
| sdk | udf-sdk | UdfContext exposes typed accessors and row iteration | `crates/exasol-udf-sdk/src/context_tests.rs` | `typed_accessors_read_the_current_row` | Pass |
| sdk | udf-sdk | UdfRun default single-call hooks return Unimplemented | `crates/exasol-udf-sdk/src/context_tests.rs` | `udf_run_spec_hooks_default_to_unimplemented` | Pass |
| sdk | udf-sdk | Spec-generation hooks receive the specification as a JSON mirror of the proto message | `crates/exa-udf-runtime/src/spec_json_tests.rs` | `spec_json_mirrors_every_proto_field` | Pass |
| sdk | udf-sdk | UdfContext reports the row count of the current input group | `crates/exasol-udf-sdk/src/context_tests.rs` | `rows_in_group_defaults_to_zero` | Pass |
| sdk | udf-sdk | UdfContext reports the declared input and output iteration axes | `crates/exasol-udf-sdk/src/context_tests.rs` | `iteration_axis_accessors_default_to_none` | Pass |
| sdk | udf-sdk | The import and export features parse the specification payload into typed structs | `crates/exasol-udf-sdk/src/spec_tests.rs` | `import_and_export_spec_parse_the_pinned_json_shape` | Pass |
| sdk | udf-sdk | The test-support feature ships a reusable UdfContext test double | `crates/exasol-udf-sdk/src/test_support_tests.rs` | `test_context_exposes_input_column_count_and_rows_in_group` | Pass |
| sdk | udf-sdk | The test-support feature ships a defaults-preserving UdfContext double | `crates/exasol-udf-sdk/src/test_support_tests.rs` | `defaults_ctx_overrides_no_provided_method` | Pass |
| sdk | udf-abi | import_spec and export_spec annotations wire the spec-generation slots | `crates/exasol-udf-macros/tests/spec_hooks.rs` | `spec_annotations_wire_both_vtable_slots` | Pass |
| sdk | udf-abi | An omitted spec annotation leaves its slot None | `crates/exasol-udf-macros/tests/spec_hooks.rs` | `omitted_spec_annotations_leave_both_slots_none` | Pass |
| sdk | udf-abi | Spec-generation vtable slots take the context pointer, bumping the ABI version | `crates/exasol-udf-sdk/src/abi_tests.rs` | `spec_slots_take_context_and_abi_version_is_ten` | Pass |
| sdk | udf-macro | name attribute overrides the SQL entry point name | `crates/exasol-udf-macros/tests/spec_hooks.rs` | `name_combines_with_spec_sections` | Pass |
| sdk | udf-macro | Macro rejects an unknown annotation section by name | `crates/exasol-udf-macros/tests/trybuild/` | `unknown_annotation_section.rs` | Pass |
| protocol | single-call | Single-call request surfaces a SingleCall host event | `crates/exa-zmq-protocol/src/loop_tests.rs` | `single_call_event_carries_both_specification_messages` | Pass |
| runtime | dispatch-single-call | An import or export spec call delivers the serialized specification to the hook | `crates/exa-udf-runtime/tests/single_call.rs` | `import_spec_call_delivers_serialized_specification` | Pass |
| runtime | dispatch-single-call | Spec-generation hooks receive a SingleCallContext | `crates/exa-udf-runtime/tests/single_call.rs` | `export_spec_hook_reads_handshake_metadata_from_context` | Pass |
| runtime | dispatch-single-call | Unimplemented single-call hook replies MT_UNDEFINED_CALL | `crates/exa-udf-runtime/tests/single_call.rs` | `undefined_call_names_the_sdk_hook` | Pass |
| runtime | dispatch-single-call | IMPORT FROM SCRIPT runs the generated SQL against its worker UDF | `crates/it/tests/db_roundtrip.rs` | `import_from_script_roundtrip` | Pass (live DB) |
| runtime | dispatch-single-call | EXPORT INTO SCRIPT surfaces the specification its hook observed | `crates/it/tests/db_roundtrip.rs` | `export_into_script_surfaces_spec` | Pass (live DB) |
| runtime | rowset-codec | InputRowSet carries the group row count of the input batch | `crates/exa-udf-runtime/src/rowset_tests.rs` | `rows_in_group_is_carried_from_the_input_batch` | Pass |
| runtime | rowset-codec | The host context reports the iteration axes the database declared | `crates/exa-udf-runtime/src/rowset_tests.rs` | `context_reports_the_declared_iteration_axes` | Pass |
| runtime | rowset-codec | The database reports a non-zero group row count over a live connection | `crates/it/tests/db_roundtrip.rs` | `rows_in_group_reports_live_group_size` | Pass (live DB) |
| examples | test-udfs | import-export-spec generates IMPORT and EXPORT SQL from the spec payload | `test-udfs/import-export-spec/src/lib_tests.rs` | `spec_hooks_build_worker_sql_from_json_spec` | Pass |
| examples | test-udfs | import-export-spec's worker reads a variadic input schema at runtime | `test-udfs/import-export-spec/src/lib_tests.rs` | `worker_builds_row_from_the_runtime_schema` | Pass |
| examples | test-udfs | rows-in-group reports the group row count the database sent | `test-udfs/rows-in-group/src/lib_tests.rs` | `reports_the_group_size_and_the_rows_it_iterated` + `reports_the_declared_count_for_an_empty_group` (renamed/split from the plan's `emits_reported_and_iterated_counts` per code-review finding 4.16 — the split isolates the non-obvious "read before iterate" invariant into its own empty-group case) | Pass |

## Notes

Two live-DB findings surfaced only by actually running the new scenarios against a real Exasol Docker DB, both fixed and re-verified green (see `specs/_plans/add-import-export-spec-hooks/tasks.md` § Phase 5 Notes for full evidence):

1. **`import_from_script_roundtrip`**: the test assumed the derived table's column aliases (`SPEC`, `PARAM_COUNT`) would reach the variadic `IMPORT_WORKER`'s reported input column names. The live engine instead names a variadic UDF's expression-derived input columns positionally (`0`, `1`); only the declared *types* survive. Confirmed not a runtime bug — `ColumnInfo` is populated verbatim from the engine's own handshake metadata (`crates/exa-udf-runtime/src/rowset.rs`), and an adjacent existing scenario (`column_metadata_reaches_the_udf`) already established that only a *declared* parameter name is preserved. Fixed by correcting the test's expected `RUNTIME_SCHEMA` constant and its comment; no production code changed.
2. **`rows_in_group_reports_live_group_size`**: the test's original SQL (`SELECT g, rows_in_group(x) FROM ... GROUP BY g`) hit a real Exasol protocol error, `Select list containing an emitting setfunction may not have additional elements` (SQL state 42000) — Exasol forbids combining an `EMITS`-declared SET UDF call with any other select-list element under `GROUP BY` (unlike a `RETURNS`-style aggregate SET UDF, which does allow it, per the pre-existing `set_sum_multi_group_by` scenario). Fixed by having the UDF itself receive and re-emit the group key (`rows_in_group(g BIGINT, x BIGINT) EMITS (g BIGINT, reported BIGINT, iterated BIGINT)`), making the UDF call the sole select-list element. This changed `test-udfs/rows-in-group/src/lib.rs` and its unit tests, not just the IT scenario.

Both corrections were applied and re-verified against the same live Exasol Docker container (`exasol/docker-db:2026.1.0`, via `testcontainers`) before this report was written; the full `db_roundtrip_all_scenarios` suite passed end-to-end on the final run.

Coverage percentage was not separately measured (`cargo llvm-cov`) for this pass; the plan's checklist does not require it. All 25 scenarios in plan.md's Scenario Coverage table map to a passing test.
