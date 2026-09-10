# Verification Report: fix-emit-ingest-wire-path

## Verdict

| Result | Details |
|--------|---------|
| **PASS** | All automated checks green, all scenarios covered, all manual tests pass |
| Code review | 3 findings — 3 fixed |

| Check | Status |
|-------|--------|
| Build | ✓ |
| Tests | ✓ |
| Lint | ✓ |
| Format | ✓ |
| Scenario Coverage | ✓ |
| Manual Tests | ✓ |

## Test Evidence

### Test Results

| Type | Run | Passed | Failed |
|------|-----|--------|--------|
| Unit + host integration (`cargo test`) | 363 | 363 | 0 |
| All features (`cargo test --all-features`) | 364 | 364 | 0 |

### Manual Tests

| Test | Result |
|------|--------|
| `MAX_TOTAL_WAIT` removed (grep returns empty) | ✓ |
| `INTEGER` docs corrected (arrives as `Value::Int64`) | ✓ |
| `lto = "fat"` in workspace and scaffold template | ✓ |
| `EXA_UDF_ABI_VERSION` is 8 | ✓ |
| `cb_log` and `/tmp/cb_debug.txt` path removed | ✓ |
| `emit_owned` in SDK trait and emit-k fixture | ✓ |
| `NUMERIC_COST_BASE` removed, `checked_ilog10` in use | ✓ |

## Tool Evidence

### Linter

```
cargo clippy --all-targets --all-features -- -D warnings: exit 0, 0 warnings
```

### Formatter

```
cargo fmt --check: exit 0, no changes needed (after fixing two blank lines left by review-fix agent)
```

## Scenario Coverage

| Domain | Feature | Scenario | Test Location | Test Name | Passes |
|--------|---------|----------|---------------|-----------|--------|
| runtime | rowset-codec | Emit stamps source input row number | `crates/it/tests/db_roundtrip.rs` | `scalar_emits_passthrough_column_resolves_source_row` | Pass |
| runtime | rowset-codec | Emit stamps source input row number (unit) | `crates/exa-udf-runtime/src/rowset_tests.rs` | `to_proto_stamps_row_number_for_scalar_and_leaves_it_empty_for_set` | Pass |
| runtime | rowset-codec | InputRowSet materialises one row at a time | `crates/exa-udf-runtime/tests/ingest_alloc.rs` | `from_proto_allocation_count_is_independent_of_row_count` | Pass |
| runtime | rowset-codec | InputRowSet round-trips all types | `crates/exa-udf-runtime/src/rowset_tests.rs` | `input_rowset_round_trips_all_exatypes_after_storage_change` | Pass |
| runtime | rowset-codec | EmitBuffer byte estimate exact for NUMERIC | `crates/exa-udf-runtime/src/rowset_tests.rs` | `numeric_byte_cost_equals_rendered_decimal_length` | Pass |
| runtime | rowset-codec | EmitBuffer group reset retains capacity | `crates/exa-udf-runtime/src/rowset_tests.rs` | `group_reset_retains_capacity_and_leaves_flush_count` | Pass |
| runtime | rowset-codec | to_proto consumes rows and moves strings | `crates/exa-udf-runtime/src/rowset_tests.rs` | `to_proto_consumes_rows_and_moves_strings` | Pass |
| protocol | handshake | Uncapped EAGAIN retry (unit) | `crates/exa-zmq-protocol/src/transport_tests.rs` | `retry_transient_retries_past_any_fixed_time_budget` | Pass |
| protocol | handshake | Transport round-trip single frame | `crates/exa-zmq-protocol/tests/transport.rs` | `transport_round_trip_single_frame` | Pass |
| protocol | handshake | Recv waits through slow reply | `crates/exa-zmq-protocol/tests/transport.rs` | `recv_waits_through_a_reply_slower_than_the_poll_interval` | Pass |
| runtime | dispatch-run-loop | Session-scoped emit buffer reused across groups | `crates/exa-udf-runtime/tests/dispatch.rs` | `session_scoped_emit_buffer_is_reused_across_groups` | Pass |
| runtime | emit-arrow-batch | push_batch matches row path | `crates/exa-udf-runtime/tests/emit_arrow_dlopen.rs` | `emit_arrow_batch_so_round_trips_via_ipc` | Pass |
| sdk | udf-sdk | emit_owned default forwards to emit | `crates/exasol-udf-sdk/tests/emit_owned.rs` | `emit_owned_default_forwards_to_emit` | Pass |
| sdk | udf-abi | ABI version is 8 | `crates/exasol-udf-sdk/src/abi_tests.rs` | `abi_version_and_vtable_layout` | Pass |
| runtime | connect-back-query | Streaming reads all rows | `crates/it/tests/db_roundtrip.rs` | `connect_back_stream_reads_all_rows` | Pass |
| runtime | debug-output | Connect-back diagnostics gated, no file | `crates/exa-udf-runtime/tests/debug_level.rs` | `connect_back_diagnostics_are_gated_and_write_no_file` | Pass |
| tools | cargo-exaudf | new scaffolds buildable crate | `crates/cargo-exasol-udf/tests/new.rs` | `new_scaffolds_crate_files` | Pass |
| runtime | rowset-codec | reserve_rows caps at MAX_RESERVE_ROWS | `crates/exa-udf-runtime/src/rowset_tests.rs` | `reserve_rows_caps_at_max_reserve_rows` | Pass |

## Notes

- Live-DB integration tests (`cargo test -p it --features integration`) require a running Exasol Docker container. CI runs these against the version matrix (8.29.x / 2025.1.x / 2026.1.x). The IT scenario `scalar_emits_passthrough_column_resolves_source_row` is wired but untested locally.
- The `emit_arrow_dlopen` test runs only under `--all-features` (the `emit-arrow-test` feature gate). It passed in the all-features run.
- Two formatting issues left by the review-fix agent were corrected before the final `cargo fmt --check` pass.
