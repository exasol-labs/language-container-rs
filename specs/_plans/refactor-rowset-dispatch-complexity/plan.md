# Plan: refactor-rowset-dispatch-complexity

## Summary

Behavior-preserving refactor resolving GitHub issue #60: remove the two production rust:S3776 CRITICAL cognitive-complexity issues (`rowset.rs:630`, `dispatch.rs:76`), cut project duplication below 3%, and close the coverage gaps in `rowset.rs`, `dispatch.rs`, and `single_call.rs`. The CLAUDE.md emit invariants are pinned as explicit tests before any restructuring; no spec changes.

## Design

### Context

SonarCloud on `main` reports 70.1% coverage, 4.0% duplication, and three CRITICAL cognitive-complexity issues. Four code smells drive this, and each is an instance of the same defect class — one design decision written down in two places (back-door leakage):

1. `compute_row_costs` (complexity 76) repeats an identical null-check-and-accumulate loop in every `DataType` arm, and re-encodes the per-type byte-width table that `value_byte_cost` already owns.
2. Sonar's 184 flagged duplicated lines in `rowset.rs` (4 blocks) are the `impl UdfContext` bodies duplicated between `HostContextBridge` and `SingleCallContext`: the handshake-delegation getters (lines 1683↔1908, 57 lines each) and the connect-back method wrappers (1819↔1982, 35 lines each).
3. `encode_slice` and `arrow_batch_to_value_rows` each re-implement the Arrow-cell-to-SDK-`Value` conversion (Arrow epoch offset, four timestamp units, decimal construction) per `ColAccessor` arm. Sonar does not flag this pair; it is conceptual leakage fixed for ownership and complexity, not for the duplication metric.
4. `dispatch.rs` and `single_call.rs` each carry their own copy of `request` (the ping-transparent REQ/REP exchange), `close_error`, and the `conn_requester` MT_IMPORT closure — the 35-line cross-file duplicated block. `run_group` (complexity 32) additionally inlines four closures/blocks whose nesting Sonar attributes to the enclosing function.

A fourth, separate finding: the uncovered lines are genuine test gaps in compiled, measured code. The CI coverage run (`cargo llvm-cov --workspace --exclude it`) builds `exa-udf-runtime` with `connect-back` (hence `emit-arrow`) enabled — `crates/exaudfclient/Cargo.toml` declares `exa-udf-runtime = { features = ["connect-back"] }`, and cargo unifies that onto the runtime's own test build in any workspace invocation. The gated code and its `arrow_tests` are compiled, run, and measured today; SonarCloud shows `compute_row_costs` entered, with specific arms uncovered (647–650, 676–696, 707–720). The job is nonetheless fragile: it builds only 3 of the 8 fixtures the runtime tests `dlopen` (the rest arrive via the cache-restored `build`-job `target/`), and the measured feature set is implicit — inherited from a sibling crate's dependency declaration. Verified 2026-08-05: `cargo test -p exa-udf-runtime --all-features` passes locally (all suites green, including `emit_arrow_dlopen`).

- **Goals** — rust:S3776 gone at `rowset.rs:630` and `dispatch.rs:76`; project duplication ≤ 3%; uncovered lines in the three files closed with targeted tests (DB-bound lines documented); wire behavior byte-identical throughout.
- **Non-Goals** — no wire-protocol, SDK-API, or spec changes; no unification of the dispatch vs. single-call loop semantics (single-call errors on unexpected events, dispatch ignores them — deliberate, kept); no coverage push for `connect_back.rs` beyond what the measurement fix reveals (out of issue scope).

### Decision

Give each duplicated decision exactly one owning module, function, or macro; decompose `run_group` into named helpers so Sonar stops attributing closure nesting to it; harden the CI coverage job (complete fixture list, explicit feature set) and close the genuine coverage gaps with targeted tests.

#### Architecture

```
crates/exa-udf-runtime/src/
├── rowset.rs
│   ├── width constants (BYTES_DATE, BYTES_TIMESTAMP, NUMERIC_COST_BASE…)
│   │     └── shared by value_byte_cost (Value axis) and fixed_cell_cost (DataType axis)
│   ├── accumulate_costs(costs, nulls, cell_cost)   ← one loop, closure-parameterized
│   │     └── compute_row_costs = fixed_cell_cost lookup + accumulate_costs per column
│   ├── accessor_value(&ColAccessor, row) -> Value  ← sole owner of Arrow-cell→Value conversion
│   │     ├── arrow_batch_to_value_rows: push(accessor_value(…))
│   │     └── encode_slice: native blocks push directly; string blocks push
│   │         value_to_block_string(&accessor_value(…))
│   └── delegation macros (delegate_handshake_meta!, delegate_connect_back_hooks!)
│         └── expand the UdfContext handshake getters and connect-back wrappers
│             once, invoked in both HostContextBridge and SingleCallContext impls
├── wire.rs  (NEW)                                  ← sole owner of one lockstep DB exchange
│   ├── request()        ping-transparent REQ/REP exchange
│   ├── close_error()    MT_CLOSE message mapping
│   └── conn_requester() MT_IMPORT credential fetcher ctor  [cfg connect-back]
├── dispatch.rs   uses wire::*; run_group decomposed into:
│   ├── emit_flusher(transport, cell) -> EmitFlusher
│   ├── batch_fetcher(transport, cell, exit) -> impl FnMut…
│   ├── drive_group_rows(bridge, udf, iter) -> Option<RuntimeError>
│   └── tail_flush(emit_buf, meta, transport, cell) -> Result<(), RuntimeError>
└── single_call.rs  uses wire::*
```

CI: the coverage step gains `--all-features` — not to widen measurement (feature unification already covers it) but to make the measured feature set explicit instead of inherited from `exaudfclient`'s dependency declaration — and the "Build test fixture cdylibs" step lists every fixture the runtime tests `dlopen` (today only 3 of 8 — the rest arrive by cache-restore luck and fail on a cold cache).

#### Patterns

| Pattern | Where | Why |
|---------|-------|-----|
| Closure-parameterized accumulator | `accumulate_costs` in `rowset.rs` | One helper serves fixed- and variable-width columns; each match arm collapses to one call |
| Single conversion owner | `accessor_value` in `rowset.rs` | Arrow epoch/unit knowledge lives once; encoders become consumers |
| Named closure constructors | `emit_flusher`, `batch_fetcher`, `conn_requester` | Moves closure nesting out of `run_group`'s cognitive-complexity attribution without changing behavior |
| Shared exchange module | `wire.rs` | Both dispatchers depend on one owner of the ping-retry and close-mapping decisions |
| Macro-owned trait delegation | `delegate_handshake_meta!`, `delegate_connect_back_hooks!` in `rowset.rs` | The duplicated `UdfContext` bodies get one textual owner; both impls invoke the macros, removing all 184 Sonar-flagged lines while keeping the per-method `#[cfg]` split and each impl's own `get`/`emit`/`next` policies hand-written |

### Consequences

| Decision | Alternatives Considered | Rationale |
|----------|------------------------|-----------|
| Generic closure accumulator for `compute_row_costs` | Per-width-class helpers (fixed vs. variable); a declarative macro | One deep helper instead of two shallow ones; a macro hides control flow without reducing measured complexity |
| New `wire.rs` module | `single_call.rs` importing from `dispatch.rs`; moving helpers into `exa-zmq-protocol` | A sibling import makes `dispatch.rs` an accidental owner; `exa-zmq-protocol` is the wrong crate — `RuntimeError` and the retry policy are runtime concerns |
| Pin the coverage feature set with explicit `--all-features` | Rely on the accidental unification from `exaudfclient` (a future feature-flag change there would silently shrink measurement); drop the flag entirely | Measurement configuration should be declared, not inherited; the flag is a no-op today and a guard tomorrow |
| Deduplicate the `UdfContext` impls with `macro_rules!` delegation | Provided defaults on the SDK trait (an SDK API change, out of scope); a shared inner struct with `Deref` (changes the impls' shape and blast radius); accept the ~5-line duplication margin from `wire.rs` alone | The macro removes all 184 flagged lines textually with zero behavior change; the issue names `rowset.rs`'s duplication explicitly, so leaving it intact would gate ≤ 3% on a 5-line margin |
| Keep dispatch vs. single-call loop semantics separate | Unify both loops over `wire.rs` | The `unexpected`-event policies differ on purpose (livelock risk in single-call); unification would change behavior |

## Features

No feature spec changes — this plan ships no spec deltas. The refactor is bound by the existing behavioral contracts of four features; their scenarios act as the protection harness (see Scenario Coverage):

| Feature | Status | Spec |
|---------|--------|------|
| runtime/rowset-codec | UNCHANGED | `specs/runtime/rowset-codec/spec.md` |
| runtime/emit-arrow-batch | UNCHANGED | `specs/runtime/emit-arrow-batch/spec.md` |
| runtime/dispatch-run-loop | UNCHANGED | `specs/runtime/dispatch-run-loop/spec.md` |
| runtime/dispatch-single-call | UNCHANGED | `specs/runtime/dispatch-single-call/spec.md` |

## Impact

None for UDF authors, DBAs, or downstream systems: no wire-format, SDK-API, or packaging change. CI-only effects: the coverage job's feature set becomes explicit (build content unchanged — unification already compiles the full matrix), its fixture list becomes cold-cache-safe, and SonarCloud metrics shift. Version bumps one patch level per the workspace versioning rule.

## Dependencies

None new. No added crates; `wire.rs` uses existing `exa-zmq-protocol`, `exa-proto`, and crate-local error types.

## Implementation Tasks

Phase ordering is normative: Phase 1 MUST be green before any Phase 3/4 restructuring lands.

### Phase 1 — Pin the emit invariants (before any restructuring)

- [ ] 1.1 Add `emit_buffer_limit_is_exactly_4_000_000` to `crates/exa-udf-runtime/src/rowset_tests.rs`: assert the literal `assert_eq!(EMIT_BUFFER_LIMIT_BYTES, 4_000_000)`. Existing tests use the constant symbolically, so a silent change to 4 MiB (4,194,304) would pass them.
- [ ] 1.2 Add a row-path mid-run flush pin in `rowset_tests.rs`: drive `HostContextBridge::emit` past the threshold with a counting `EmitFlusher`; assert exactly one mid-run flush fires and residual rows remain buffered for the tail flush. (The batch path has `bridge_emit_batch_buffers_and_flushes`; the row path today pins only buffer-level `should_flush`.)
- [ ] 1.3 Record the green baseline: `cargo test --workspace` and `cargo test -p exa-udf-runtime --all-features` (build fixtures first: `cargo build -p scalar-double -p annotated-fixture -p single-call-fixture`). Existing invariant pins that MUST stay green throughout: `emit_buffer_byte_estimate_and_should_flush`, `oversized_single_row_flushes_alone`, `emit_buffer_spans_group_and_tail_flushes`, `push_batch_splits_oversized_batch`, `push_batch_equals_row_push`, `fast_path_to_proto_byte_identical_to_row_path`.

### Phase 2 — Harden the coverage job (CI; no measurement change)

- [ ] 2.1 In `.github/workflows/ci.yml`: extend the "Build test fixture cdylibs" step to every fixture the runtime tests `dlopen`: `scalar-double`, `annotated-fixture`, `single-call-fixture`, `set-sum`, `emit-k`, `scalar-next-illegal`, `returns-with-emit`, `emit-arrow-batch`. The gated suites (`emit_arrow_dlopen`, `connect_back`) already run in this job via feature unification from `exaudfclient`; today's 3-fixture list survives only because the cache restores the `build` job's `target/`, and a cold cache fails. Additionally add `--all-features` to the `cargo llvm-cov` invocation (keep `--workspace --exclude it`) — a build no-op today, it pins the measured feature set explicitly instead of inheriting it from `exaudfclient`'s dependency declaration.

### Phase 3 — rowset.rs refactor (sequential within phase; parity tests green after each task)

- [ ] 3.1 Extract named width constants shared by `value_byte_cost` and a new `fixed_cell_cost(&DataType) -> Option<usize>`; add `accumulate_costs(costs: &mut [usize], nulls: Option<&NullBuffer>, cell_cost: impl Fn(usize) -> usize)`; rewrite `compute_row_costs` over them (fixed-width arms collapse to one lookup + one call; `Utf8`/`LargeUtf8` pass a length closure). Parity guard: `push_batch_byte_estimate_parity`.
- [ ] 3.2 Extract `accessor_value(&ColAccessor, row) -> Value` as the single Arrow-cell→`Value` conversion; rewrite `arrow_batch_to_value_rows` to push it directly and `encode_slice` to route string-block accessors through `value_to_block_string(&accessor_value(…))` while native-block accessors (`Int32`/`Int64`/`Float64`/`Boolean`) keep direct pushes; `Unsupported` maps to `Value::Null` / skip exactly as today. Output MUST stay byte-identical. Guards: `push_batch_equals_row_push`, `push_batch_null_bitmap`, `push_batch_shared_block_type_interleaved`, `push_batch_int64_into_numeric_block`, `fast_path_to_proto_byte_identical_to_row_path`. [expert]
- [ ] 3.3 Add direct unit tests in `rowset_tests.rs` (hand-built Arrow batches): `compute_row_costs` per-type widths agree with `value_byte_cost` (including `Decimal128` scale term), NULL cells cost 0, multi-column rows sum; `accessor_value` edges — the four timestamp units, `Date32` epoch offset, `NumericFromInt32/Int64/Float64`, `Unsupported` → `Value::Null`; `build_accessors` type-mismatch error arms.
- [ ] 3.4 Deduplicate the `impl UdfContext` bodies (Sonar's 184 flagged lines): define `delegate_handshake_meta!` (the `memory_limit` … `scope_user` getters plus `debug_level`) and `delegate_connect_back_hooks!` (the `#[cfg(feature = "connect-back")]` `cluster_ip`/`connection`/`connect_back` wrappers with their `record_error` calls) as `macro_rules!` in `rowset.rs`; invoke both macros from the `HostContextBridge` and `SingleCallContext` impls. Each impl's differing `get`/`emit`/`next`/`set_return` policies stay hand-written; the per-method `#[cfg]` split is preserved inside the macro. Guards: `bridge_returns_handshake_metadata`, `single_call_context_returns_handshake_metadata`, `host_bridge_debug_level_returns_valid_level`, `single_call_context_debug_level_returns_valid_level`, plus `cargo clippy --all-targets --all-features`.

### Phase 4 — dispatch.rs / single_call.rs refactor (sequential within phase)

- [ ] 4.1 Create `crates/exa-udf-runtime/src/wire.rs` with `pub(crate) request()`, `pub(crate) close_error()`, and `#[cfg(feature = "connect-back")] pub(crate) conn_requester<'a>(&'a ZmqTransport, &'a RefCell<&'a mut Protocol>) -> ConnRequester<'a>`; delete the duplicates from `dispatch.rs` and `single_call.rs` and update call sites. Keep `single_call.rs`'s `unexpected` where it is — it is policy, not plumbing.
- [ ] 4.2 Decompose `run_group` into named helpers — `emit_flusher`, `batch_fetcher`, `drive_group_rows` (the `IterType` match with the scalar per-row loop), `tail_flush` — preserving the `RefCell<&mut Protocol>` non-overlapping-borrow discipline and the `Cell<Option<GroupExit>>` exit signaling documented in the current comments; `run_group` becomes sequential composition with cognitive complexity < 15. [expert]
- [ ] 4.3 Add mock-DB tests (existing REP-socket harness in `crates/exa-udf-runtime/tests/dispatch.rs` and `tests/single_call.rs`) for the gaps the extraction exposes: mid-group `MT_CLEANUP` (clean session end) and mid-group `MT_CLOSE` (surfaced message) via the `GroupExit` paths; `invoke_run` rc != 0 with and without out-pointer error text; ping-pong mid-exchange retry through `wire::request`; unexpected-event hard error in single-call mode.

### Phase 5 — Close the genuine coverage gaps (315 + 54 + 40 baseline lines)

- [ ] 5.1 Measure: run `cargo llvm-cov -p exa-udf-runtime -p exaudfclient --show-missing-lines` (the `-p exaudfclient` reproduces CI's unified feature set) and enumerate the lines still uncovered in `rowset.rs`, `dispatch.rs`, and `single_call.rs` after Phases 3–4.
- [ ] 5.2 Close the `rowset.rs` residuals (315-line baseline): the pre-refactor uncovered `compute_row_costs` arms (647–650, 676–696, 707–720) and their post-refactor equivalents not already closed by 3.3; `encode_slice`/`arrow_batch_to_value_rows` arms the parity tests miss; fast-parser defer/error branches, `decode_string_block` leniency arms, `fast_decimal_to_string` branches; `HostContextBridge`/`SingleCallContext` method gaps; `request_connection` via an injected fake `ConnRequester`. Lines requiring a live DB (`open_connect_back`'s exarrow session establishment) are out of unit reach — list them explicitly in the verification report.
- [ ] 5.3 Close the `dispatch.rs` (54-line baseline) and `single_call.rs` (40-line baseline) residuals beyond task 4.3's tests, using the same mock-DB harness.

### Phase 6 — Verification and release chores

- [ ] 6.1 Full checklist below, including `cargo test -p it --features integration` against a local Exasol Docker DB.
- [ ] 6.2 Bump `[workspace.package].version` (patch), update the pinned `exasol-udf-sdk` entry in `[workspace.dependencies]` to match, regenerate and commit `Cargo.lock` in the same PR. Rebuild local test-udf `.so`s afterward (fingerprint tracks the version).

## Parallelization

| Parallel Group | Tasks |
|----------------|-------|
| Group A (first) | 1.1, 1.2, 1.3 |
| Group B | 2.1 |
| Group C | 3.1 → 3.2 → 3.3 → 3.4 (sequential, same file) |
| Group D | 4.1 → 4.2 → 4.3 (sequential, same files) |
| Group E | 5.1 → (5.2 ∥ 5.3) |
| Group F | 6.1, 6.2 |

Sequential dependencies:
- Group A → C and A → D (invariant pins precede restructuring)
- Groups B, C, D are mutually independent (disjoint files) and run concurrently
- B, C, D → E (residual coverage is measured against the refactored code)
- Within E: 5.1 first; 5.2 and 5.3 then run concurrently (disjoint files)
- E → F

## Dead Code Removal

All removals are replace-in-place; nothing is orphaned.

| Type | Location | Reason |
|------|----------|--------|
| Function | `crates/exa-udf-runtime/src/dispatch.rs::request` | Replaced by `wire::request` |
| Function | `crates/exa-udf-runtime/src/dispatch.rs::close_error` | Replaced by `wire::close_error` |
| Function | `crates/exa-udf-runtime/src/single_call.rs::request` | Replaced by `wire::request` |
| Function | `crates/exa-udf-runtime/src/single_call.rs::close_error` | Replaced by `wire::close_error` |
| Closure | `conn_requester` inline in `dispatch.rs::run_group` and `single_call.rs::invoke_vs_adapter_call` | Replaced by `wire::conn_requester` |
| Code blocks | Per-`DataType` loops in `rowset.rs::compute_row_costs` | Replaced by `accumulate_costs` |
| Code blocks | Per-`ColAccessor` conversion arms duplicated across `encode_slice` and `arrow_batch_to_value_rows` | Replaced by `accessor_value` |
| Code blocks | `UdfContext` handshake-delegation getters and connect-back wrappers duplicated across the `HostContextBridge` and `SingleCallContext` impls (`rowset.rs` 1683↔1908, 1819↔1982) | Replaced by `delegate_handshake_meta!` / `delegate_connect_back_hooks!` invocations |

## Verification

### Scenario Coverage

All scenarios below are the existing contracts of the four bounding features. "existing" tests are the protection harness and MUST stay green through every phase; "NEW" tests are added by the task named.

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| rowset-codec / EmitBuffer packs output values row-major by declared column type | Unit | `crates/exa-udf-runtime/src/rowset_tests.rs` | `emit_packs_by_declared_type_not_value_variant` (existing) |
| rowset-codec / InputRowSet decodes row-major type blocks correctly | Unit | `crates/exa-udf-runtime/src/rowset_tests.rs` | `bridge_materializes_input_rows` (existing) |
| rowset-codec / A single emitted row larger than the flush threshold is sent on its own | Unit + Integration | `crates/exa-udf-runtime/src/rowset_tests.rs`; `crates/it/tests/db_roundtrip.rs` | `oversized_single_row_flushes_alone` (existing); `emit_bulk_boundary_rows_and_oversize_row` (existing) |
| rowset-codec / EmitBuffer tracks a running byte estimate and reports when to flush | Unit | `crates/exa-udf-runtime/src/rowset_tests.rs` | `emit_buffer_byte_estimate_and_should_flush` (existing); `emit_buffer_limit_is_exactly_4_000_000` (NEW, task 1.1) |
| rowset-codec / EmitBuffer emits timestamps at full nanosecond precision | Unit | `crates/exa-udf-runtime/src/rowset_tests.rs` | `timestamp_emit_nanosecond_roundtrip` (existing) |
| rowset-codec / A promoted emit fast-path encoder stays byte-identical to the row path | Unit | `crates/exa-udf-runtime/src/rowset_tests.rs` | `fast_path_to_proto_byte_identical_to_row_path` (existing) |
| rowset-codec / A promoted ingest fast-path decoder round-trips byte-identically | Unit | `crates/exa-udf-runtime/src/rowset_tests.rs` | `fast_parse_date_matches_chrono_parse_for_valid_dates`, `fast_parse_timestamp_matches_chrono_parse_for_valid_timestamps` (existing) |
| emit-arrow-batch / EmitBuffer encodes an Arrow batch column-at-a-time into proto type blocks | Unit | `crates/exa-udf-runtime/src/rowset_tests.rs` | `push_batch_equals_row_push`, `push_batch_shared_block_type_interleaved` (existing) |
| emit-arrow-batch / push_batch splits an oversized batch at row boundaries under the 4 MB cap | Unit | `crates/exa-udf-runtime/src/rowset_tests.rs` | `push_batch_splits_oversized_batch` (existing) |
| emit-arrow-batch / push_batch produces proto blocks identical to the row-based push path | Unit | `crates/exa-udf-runtime/src/rowset_tests.rs` | `push_batch_equals_row_push`, `push_batch_byte_estimate_parity` (existing) |
| emit-arrow-batch / Bridge deserialises emit_batch IPC bytes, carries the output metadata, and flushes on the same threshold | Integration | `crates/exa-udf-runtime/tests/emit_arrow_dlopen.rs` | `emit_arrow_batch_so_round_trips_via_ipc` (existing) |
| emit-arrow-batch / emit and emit_batch share one buffer and one tail flush | Unit | `crates/exa-udf-runtime/src/rowset_tests.rs` | `bridge_mixed_emit_styles_share_buffer` (existing) |
| dispatch-run-loop / Bridge materializes input rows into typed accessors | Unit | `crates/exa-udf-runtime/src/rowset_tests.rs` | `bridge_materializes_input_rows`, `bridge_typed_accessors` (existing) |
| dispatch-run-loop / Scalar dispatch invokes the UDF once per input row | Integration | `crates/exa-udf-runtime/tests/dispatch.rs` | `scalar_dispatch_invokes_run_per_row` (existing) |
| dispatch-run-loop / Set dispatch invokes the UDF once per group spanning all input batches | Integration | `crates/exa-udf-runtime/tests/dispatch.rs` | `set_dispatch_next_spans_batches` (existing) |
| dispatch-run-loop / Scalar input context rejects next() | Integration | `crates/exa-udf-runtime/tests/dispatch.rs` | `scalar_next_returns_error` (existing) |
| dispatch-run-loop / RETURNS output emits the value the UDF returned and bans emit() | Integration | `crates/exa-udf-runtime/tests/dispatch.rs` | `returns_set_return_and_emit_ban` (existing) |
| dispatch-run-loop / Compiled output shape is validated against the DB output iteration type | Integration | `crates/exa-udf-runtime/tests/dispatch.rs` | `output_shape_marker_mismatch_errors` (existing) |
| dispatch-run-loop / Emit buffer spans an input group across per-row and per-batch iteration | Integration | `crates/exa-udf-runtime/tests/dispatch.rs` | `emit_buffer_spans_group_and_tail_flushes` (existing) |
| dispatch-run-loop / UDF error closes the session with a prefixed message | Integration | `crates/it/tests/db_roundtrip.rs`; `crates/exa-udf-runtime/tests/dispatch.rs` | `udf_error_surfaces_prefix` (existing); `udf_error_closes_session_with_prefixed_message` (NEW, task 4.3) |
| dispatch-run-loop / Dispatch reads UDF error text from the run out-pointer | Integration | `crates/exa-udf-runtime/tests/dispatch.rs` | `run_error_out_pointer_text_reaches_close` (NEW, task 4.3) |
| dispatch-run-loop / Connect-back is available identically in scalar and set dispatch | Integration | `crates/it/tests/db_roundtrip.rs` | `connect_back_scalar_queries_and_returns`, `connect_back_udf_queries_and_emits` (existing) |
| dispatch-run-loop / Bridge surfaces handshake identity and origin metadata to the UDF | Unit | `crates/exa-udf-runtime/src/rowset_tests.rs` | `bridge_returns_handshake_metadata` (existing) |
| dispatch-single-call / Single-call mode routes to the single-call dispatcher | Integration | `crates/exa-udf-runtime/tests/single_call.rs` | `single_call_mode_routes_to_dispatcher` (existing) |
| dispatch-single-call / Single-call dispatch invokes the matching vtable hook and returns | Integration | `crates/exa-udf-runtime/tests/single_call.rs` | `dispatch_invokes_default_output_columns` (existing) |
| dispatch-single-call / Unimplemented single-call hook replies MT_UNDEFINED_CALL | Integration | `crates/exa-udf-runtime/tests/single_call.rs` | `unimplemented_hook_replies_undefined_call` (existing) |
| dispatch-single-call / Virtual-schema adapter call is dispatched to the adapter hook | Integration | `crates/exa-udf-runtime/tests/single_call.rs`; `crates/it/tests/db_roundtrip.rs` | `dispatch_surfaces_adapter_hook_error` (existing); `single_call_adapter_surfaces_live_handshake_metadata` (existing) |
| dispatch-single-call / Annotated schema is validated against the database metadata at load | Integration | `crates/exa-udf-runtime/tests/dispatch.rs` | `annotated_schema_mismatch_closes_session` (existing) |
| dispatch-single-call / Single-call hook error text is surfaced when rc != 0 | Integration | `crates/exa-udf-runtime/tests/single_call.rs` | `dispatch_surfaces_adapter_hook_error` (existing) |
| dispatch-single-call / Adapter single-call context surfaces live handshake metadata | Unit | `crates/exa-udf-runtime/src/rowset_tests.rs` | `single_call_context_returns_handshake_metadata` (existing) |

### Manual Testing

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| rowset-codec / emit-arrow-batch | `cargo test -p exa-udf-runtime --all-features` (after `cargo build -p scalar-double -p annotated-fixture -p single-call-fixture`) | All suites pass, 0 failures, including `arrow_tests` and `emit_arrow_dlopen` |
| dispatch-run-loop | `cargo test -p it --features integration` (local Exasol Docker DB) | 0 failures; emit/scalar/set roundtrips green, incl. `emit_bulk_boundary_rows_and_oversize_row` |
| dispatch-single-call | `cargo test -p exa-udf-runtime --test single_call` | 5+ tests pass against the mock REP socket |
| Coverage closure | `cargo llvm-cov -p exa-udf-runtime -p exaudfclient --show-missing-lines` | No missing lines reported for `rowset.rs`, `dispatch.rs`, `single_call.rs` except the DB-bound lines listed in the verification report |

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Build | `cargo build --release` | Exit 0 |
| Test | `cargo test` | 0 failures |
| All-features test | `cargo test -p exa-udf-runtime --all-features` | 0 failures |
| Integration | `cargo test -p it --features integration` | 0 failures (fails, not skips, without Docker DB) |
| Lint | `cargo clippy --all-targets --all-features -- -D warnings` | 0 errors/warnings |
| Format | `cargo fmt --check` | No changes |

Sonar acceptance (rust:S3776 issues gone, duplication ≤ 3%) is confirmed on SonarCloud after the PR's CI run; there is no exact local Sonar oracle. Local proxy: the refactored functions compose sequentially with no nested control flow beyond one level.

Duplication arithmetic: today 254 duplicated / 6,324 lines = 4.0%. The `wire.rs` extraction (task 4.1) removes the 70-line dispatch/single_call pair → ≈ 184/6,300 ≈ 2.9%, under the gate by only ~5 lines. Task 3.4 removes the remaining 184 flagged `rowset.rs` lines, taking this scope's contribution to ≈ 0 and the project comfortably under the issue's "well under 2%" expectation — the gate does not rest on a 5-line margin.
