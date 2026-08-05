# Tasks: refactor-rowset-dispatch-complexity

## Phase 1: Pin emit invariants (Group A, sequential)
- [x] 1.1 Add `emit_buffer_limit_is_exactly_4_000_000` to `rowset_tests.rs`
- [x] 1.2 Add row-path mid-run flush pin in `rowset_tests.rs`
- [x] 1.3 Record green baseline (cargo test --workspace; cargo test -p exa-udf-runtime --all-features)

## Phase 2: Harden coverage job (Group B, independent of C/D)
- [x] 2.1 Extend CI fixture list + add --all-features to llvm-cov in .github/workflows/ci.yml

## Phase 3: rowset.rs refactor (Group C, sequential within phase)
- [x] 3.1 Extract width constants, fixed_cell_cost, accumulate_costs; rewrite compute_row_costs
- [x] 3.2 Extract accessor_value; rewrite arrow_batch_to_value_rows/encode_slice [expert]
- [x] 3.3 Add direct unit tests for compute_row_costs / accessor_value / build_accessors
- [x] 3.4 Deduplicate impl UdfContext bodies via delegate_handshake_meta!/delegate_connect_back_hooks! macros

## Phase 4: dispatch.rs / single_call.rs refactor (Group D, sequential within phase)
- [x] 4.1 Create wire.rs (request, close_error, conn_requester); delete duplicates, update call sites
- [x] 4.2 Decompose run_group into emit_flusher, batch_fetcher, drive_group_rows, tail_flush [expert]
- [x] 4.3 Add mock-DB tests for gaps exposed by extraction

## Phase 5: Close genuine coverage gaps (Group E)
- [x] 5.1 Measure: cargo llvm-cov -p exa-udf-runtime -p exaudfclient --show-missing-lines
- [x] 5.2 Close rowset.rs coverage residuals
- [x] 5.3 Close dispatch.rs / single_call.rs coverage residuals

## Phase 6: Verification and release chores (Group F)
- [x] 6.1 Full checklist incl. cargo test -p it --features integration
- [x] 6.2 Bump workspace version (patch); update exasol-udf-sdk pin; regen Cargo.lock; rebuild test-udf .so's

## Phase 7: Review Fixes
<!-- Indices continue past Phase 6 because `## Phase 4` is already the
     dispatch/single_call refactor phase and 4.1/4.2 are taken. -->
- [x] 7.1 Split `accessor_value`'s `TsNanosecond` count with `div_euclid`/`rem_euclid` so a negative `ns` decodes to its pre-epoch instant; retarget `accessor_value_timestamp_nanosecond_negative_degrades_to_epoch` to that contract [expert]
- [x] 7.2 Rewrite `wire::request`'s ping retry as a loop and extend `ping_pong_mid_exchange_retries_transparently` to two consecutive pings with distinct tokens [expert]
- [x] 7.3 Delete the unmeasured "+25%" benchmark claim from `value_into_block_string`'s doc comment (rowset.rs) and replace it with the structural rationale (borrowing would double-clone `Value::String`; the pre-refactor code moved the Arrow `to_string()` result)
- [x] 7.4 Fix `encode_slice`'s doc comment (rowset.rs) to name `value_into_block_string` as the string-block renderer, noting it moves rather than copies a `Value::String` and yields the same bytes as `value_to_block_string` for every variant
- [x] 7.5 Rewrite the three network-dependent tests in rowset_tests.rs (`first_nonloopback_ipv4_returns_a_valid_non_loopback_ipv4_address`, `bridge_cluster_ip_delegates_to_first_nonloopback_ipv4`, `single_call_context_cluster_ip_delegates_to_first_nonloopback_ipv4`) to be deterministic on loopback-only hosts
- [x] 7.6 Add a `start_mock_session` helper to tests/dispatch.rs and replace the nine hand-rolled mock-DB bring-ups with calls to it; delete the now-unused `scalar_so_path` helper
- [x] 7.7 Add a profile-independent panic arm for `Value::Int64(i64::MAX)` in test-udfs/scalar-double/src/lib.rs and update `udf_error_closes_session_with_prefixed_message`'s comment in tests/dispatch.rs to cite it instead of dev-profile overflow-checks
- [x] 7.8 Rewrite the module doc's final sentence in test-udfs/single-call-fixture/src/lib.rs to state which three hooks are wired and that only `generate_sql_for_export_spec` is left `None`
