# Plan: fix-emit-ingest-wire-path

## Summary

Close all eleven findings of issue #95: two correctness defects on the emit/ingest wire path (the never-echoed `row_number` and the 120 s transport cap), the copy and allocation reductions on emit and ingest, and the docs, build-profile and benchmark gaps that hide the cost.

## Design

### Context

The Rust SLC's emit and ingest path was ported for correctness of the message sequence, not for the copy count. A review against the engine source (`Engine/src/exscript/pluggable/{zmqcontainer.cc,zmqinternal.cc,swigcontainers_int.h}`, `Main/global/engine/exscript/vmchelpers.h`) found eleven divergences from the reference C++ client. Two are correctness defects. Seven are per-row costs the engine does not pay. Two are measurement and documentation gaps that let the costs hide.

The two defects are unrelated in mechanism and share no code. Both emit an empty `row_number` array (`rowset.rs:365`, `rowset.rs:886`), so the engine resolves every pass-through output column against an out-of-range index with no bounds check. And `MAX_TOTAL_WAIT` aborts the VM after 120 s of `EAGAIN` polling, while the engine's own retry loop never gives up on time.

The cost findings concentrate in three files. `rowset.rs` carries four user-space copies of every emitted string and three plus one allocation per ingested row. `transport.rs` copies each frame into and out of libzmq. `dispatch.rs` allocates a buffer, a cell and three boxed closures per input group. `connect_back.rs` materialises a whole result set and appends a diagnostic file per call.

- **Goals** — correct pass-through output columns; remove the spurious transport timeout; cut the per-row copy and allocation count on emit and ingest; make the NUMERIC flush estimate exact; give authors and the scaffold a release profile that inlines; document the fast and slow SQL types; measure the native-integer ceiling.
- **Non-Goals** — no change to the wire byte format apart from populating `row_number`; no change to the 4,000,000-byte flush threshold or the `MT_RUN`/`MT_NEXT`/`MT_DONE`/`MT_CLEANUP` sequencing; no change to the hand-rolled DATE/TIMESTAMP/DECIMAL formatters; no columnar transport; no change to `ExaConnection`'s Arrow-free boundary.

### Decision

#### Architecture

Ownership, not structure, is the change. Each stage of the emit path stops copying what it can move, and the ingest path stops materialising what it can borrow.

```
emit, before                                  emit, after
 ctx.emit(&[Value])                             ctx.emit_owned(Vec<Value>)
   └─ values.to_vec()          clone 1            └─ (move)
   └─ to_proto(&self)          clone 2            └─ to_proto(&mut self)   move
   └─ encode_to_vec()          copy  3            └─ encode_to_vec()       copy 1
   └─ socket.send(&buf)        copy  4            └─ send(Message::from)   move
                                                                    4 copies → 1

ingest, before                                ingest, after
 recv_bytes()                  copy  1          recv_msg()               borrow
   └─ prost decode             copy  2            └─ prost decode        copy 1
   └─ from_proto(&table)       copy  3            └─ from_proto(table)    move
   └─ Vec<Value> per row       alloc/row          └─ one scratch Vec      alloc/batch
```

`InputRowSet` changes its storage model. Today it decodes a whole batch into a dense `Vec<Vec<Value>>`. After the change it owns the proto typed arrays and materialises the current row into one reusable scratch `Vec<Value>`. `get` keeps returning `&Value` and `current_row` keeps returning `&[Value]`, so no caller signature changes. The random-access `row(idx)` accessor becomes forward-only `seek_row(idx)`, because random access is what would force the storage to carry per-row cursor offsets that no production caller reads.

`row_number` flows the same way input rows do. `InputRowSet` retains the array the engine fills on each `MT_NEXT`. The bridge stamps the current input row's number on each buffered output row while the input axis is `ExactlyOnce`, and leaves the array empty for `Multiple`, where a SET group has no single source row.

#### Patterns

| Pattern | Where | Why |
|---------|-------|-----|
| Move-through ownership on each emit stage | `UdfContext::emit_owned` → `EmitBuffer` → `to_proto` → `zmq::Message` | Each copy exists only because a stage borrows what the next stage keeps |
| Proto arrays as storage, one scratch row | `InputRowSet` | Removes one heap allocation per ingested row while `get` still returns `&Value` |
| Re-encode on retry, not keep a spare copy | `ZmqTransport::send` | `Socket::send` consumes the message, so the alternative is a second copy of every frame on the successful path |
| Session-scoped buffer and closures with a per-group reset | `dispatch::run_udf` / `run_group` | Fixed per-group cost is the only lever left for a SET session of many small groups |
| Exact digit count for the NUMERIC estimate | `value_byte_cost` | `checked_ilog10` on the unscaled magnitude is O(1) and exact, so no accuracy is traded for speed |
| Capped reservation from `rows_in_group` | emit buffer pre-size | `rows_in_group` is unbounded; an uncapped reserve would breach the per-instance memory limit |
| Structured tracing field instead of a diagnostic file | `connect_back.rs` | Gates the line by `%udf_debug_level` and removes three syscalls per connect-back call |

#### Design Diagnostic

`emit_owned` is the only new interface and `InputRowSet`'s row surface the only changed boundary, so both answer `/speq:design-philosophy`'s Quick Diagnostic.

`emit_owned` is a second method for one concept, which normally reflects an implementation decision into the author API. It earns its place because the alternative is a breaking change to `emit`, and its doc comment states the design intent (move versus clone) rather than restating the name. It carries a forwarding default, so the ownership decision stays invisible to any `UdfContext` implementation that does not care.

`InputRowSet`'s interface narrows rather than widens. Callers gain nothing to learn and lose random access they never used. The storage decision (proto arrays plus a scratch row) stays inside the type: `get`, `current_row`, `advance`, `len` and `is_empty` keep their signatures, so nothing outside the type learns how a row is materialised.

### Consequences

| Decision | Alternatives Considered | Rationale |
|----------|------------------------|-----------|
| Remove the transport cap entirely | Raise the cap to a larger value and log the wait | The engine's own retry loop has no time limit. Any finite cap is a guess that turns a slow query into a VM crash. The database watchdog already ends wedged sessions |
| Full `InputRowSet` storage redesign now | Ship only the immediate fix (`recv_msg` slice, table by value, `into_iter` drain) | The immediate fix removes two copies but leaves one heap allocation per row, the dominant remaining ingest cost |
| Re-encode the request on an `EAGAIN` send retry | Keep the encoding buffer alive alongside the message | Keeping the buffer restores the copy the change removes. `EAGAIN` on send is rare, so the re-encode cost falls where it does not matter |
| Forward-only `seek_row` replacing `row(idx)` | Precompute per-row cursor offsets to keep random access | Offsets are state only tests read. Shaping the storage around a test's convenience is what the interface is meant to prevent |
| `Decimal128` costed from declared precision | Reuse the exact per-value digit formula | The Arrow path costs a column by declared type and has no cell value to measure. `precision + 2` is the exact ceiling that type can produce |
| Cap the `rows_in_group` reservation | Reserve `rows_in_group` verbatim, as the issue proposes | `rows_in_group` is the group's total input row count and is unbounded. A 100M-row group would reserve gigabytes of row headers |
| Remove the duplicate connect-back streaming scenario | Change both scenarios in parallel | The two scenarios state one requirement with different mechanism words. Editing both would leave the wordings in conflict |

## Features

| Feature | Status | Spec |
|---------|--------|------|
| runtime/rowset-codec | CHANGED | `runtime/rowset-codec/spec.md` |
| protocol/handshake | CHANGED | `protocol/handshake/spec.md` |
| runtime/dispatch-run-loop | CHANGED | `runtime/dispatch-run-loop/spec.md` |
| runtime/emit-arrow-batch | CHANGED | `runtime/emit-arrow-batch/spec.md` |
| sdk/udf-sdk | CHANGED | `sdk/udf-sdk/spec.md` |
| sdk/udf-abi | CHANGED | `sdk/udf-abi/spec.md` |
| runtime/connect-back-query | CHANGED | `runtime/connect-back-query/spec.md` |
| runtime/debug-output | CHANGED | `runtime/debug-output/spec.md` |
| tools/cargo-exaudf | CHANGED | `tools/cargo-exaudf/spec.md` |
| examples/test-udfs | CHANGED | `examples/test-udfs/spec.md` |

## Impact

A SCALAR UDF used alongside other select-list expressions returns correct pass-through column values. Before this change those columns read whatever the engine found at an out-of-range `row_number` index, or the query crashed.

**Breaking change: `EXA_UDF_ABI_VERSION` goes 7 → 8.** Adding `emit_owned` to `UdfContext` widens the trait-object vtable. Every previously built UDF `.so` must be rebuilt against the new SDK. The loader reports `AbiMismatch` for a stale artifact rather than misdispatching. This falls inside the existing rebuild cadence: `sdk_fingerprint` already changes on every version bump, so a stale `.so` was already rejected.

UDF source code compiles unchanged. `emit` keeps its signature, and `emit_owned` carries a forwarding default, so custom `impl UdfContext` test doubles need no edit.

A query that the 120 s transport cap used to abort now runs to completion. A genuinely wedged session now waits for the database watchdog instead of self-aborting.

Connect-back sessions no longer write `/tmp/cb_debug.txt`. Anyone reading that file for diagnostics must read the UDF stderr log at `%udf_debug_level` debug instead.

Authoring guidance changes: `docs/writing-a-udf.md` claimed `INT`/`INTEGER` arrive as `Decimal`. They are `DECIMAL(18,0)` and arrive as `Value::Int64`. An author following the old text annotated `Decimal` on an `INTEGER` column and hit a load-time validation error.

## Dependencies

None external. Task 1.7 depends on the `sdk/udf-abi` bump landing in the same change, because the vtable widens and the version guards it.

## Implementation Tasks

Order follows the issue's ranking: correctness first, then the small measurable wins, then the shared ownership refactor, then docs, build and bench, then the independent items.

### 1. Emit and ingest codec (F1, F5, F3, F4, F8)

- [ ] 1.1 Retain the `MT_NEXT` batch's `row_number` array in `InputRowSet` and expose the current row's number. [F1]
- [ ] 1.2 Stamp the current input row's number onto each row `push_output_row` buffers when the input axis is `ExactlyOnce`, serialise the array in `to_proto`, and keep it empty for `Multiple` and for a batch that supplies fewer entries than rows. [F1] [expert]
- [ ] 1.3 Fill `row_number` in `encode_slice` under the same rule, so the Arrow batch path keeps byte parity with the row path. [F1]
- [ ] 1.4 Add IT scenario `scalar_emits_passthrough_column_resolves_source_row`: `SELECT k, emit_k(k) FROM it_rust.emit_k_src` asserting each pass-through value equals its source input, then repeat over `ORDINAL_100K` so the assertion crosses a scalar `MT_NEXT` batch boundary. [F1]
- [ ] 1.5 Replace `NUMERIC_COST_BASE` with the exact rendered-decimal length from `checked_ilog10`, and cost Arrow `Decimal128(p, s)` as `p + 2`. [F5]
- [ ] 1.6 Change `to_proto` to `&mut self`, consume the buffered rows, and move strings through `value_into_block_string`. [F3]
- [ ] 1.7 Add `emit_owned(Vec<Value>)` to `UdfContext` with a forwarding default, override it in `HostContextBridge` to move the row, bump `EXA_UDF_ABI_VERSION` 7 → 8, and switch the `emit-k` fixture to `emit_owned` so the new vtable slot is exercised end to end. [F3] [expert]
- [ ] 1.8 Restructure `InputRowSet` to keep the proto typed arrays as storage: take the table by value, drain `data_string` with `into_iter`, materialise the current row into one reusable scratch `Vec<Value>`, and replace `row(idx)` with forward-only `seek_row(idx)`. [F4] [expert]
- [ ] 1.9 Hoist the `EmitBuffer`, the `Protocol` cell, the emit flusher and the batch fetcher from `run_group` to session scope in `run_udf`, add the capacity-retaining group reset that leaves `flush_count` untouched, and reserve from `rows_in_group` under a constant row cap. [F8] [expert]
- [ ] 1.10 Add `crates/exa-udf-runtime/tests/ingest_alloc.rs` with a counting global allocator, asserting the allocation count of `from_proto` plus a full row walk does not grow with the batch's row count. [F4]

### 2. ZMQ transport frames (F2, F3, F4)

- [ ] 2.1 Remove `MAX_TOTAL_WAIT`, its elapsed-time branch and the timeout `ProtocolError`, and extract the retry loop into a socket-free helper so it is unit-testable. [F2]
- [ ] 2.2 Send the encoded frame as `zmq::Message::from(Vec<u8>)`, re-encoding the request on each `EAGAIN` retry attempt. [F3]
- [ ] 2.3 Decode the response from `recv_msg`'s byte slice instead of `recv_bytes`. [F4]
- [ ] 2.4 Add `crates/exa-zmq-protocol/src/transport_tests.rs` (declared as the last item of `transport.rs`) covering the uncapped retry, and extend `crates/exa-zmq-protocol/tests/transport.rs` for the message-based send and slice-based decode. [F2, F3, F4]

### 3. Connect-back streaming and diagnostics (F9, F10)

- [ ] 3.1 Drive batch iteration inside the single `block_on` async block, invoke the row callback from there, and drop `fetch_all`. [F9] [expert]
- [ ] 3.2 Replace `cb_log` with `tracing::debug!` carrying the SQL as a structured field, and delete the file-append path. [F10]
- [ ] 3.3 Extend `crates/exa-udf-runtime/tests/debug_level.rs` to assert the connect-back events are level-gated and that no diagnostic file is created. [F10]

### 4. Documentation, build profile and benchmark (F6, F7, F11)

- [ ] 4.1 Correct the integer mapping in `docs/writing-a-udf.md`: `INT`/`INTEGER` is `DECIMAL(18,0)` and arrives as `Value::Int64`; only `BIGINT` (`DECIMAL(36,0)`) arrives as `Value::Numeric`. Fix the annotation guidance at line 170 and the `Value` reference note at line 217. [F6]
- [ ] 4.2 Add a performance section to `docs/writing-a-udf.md`: prefer `DECIMAL(18,0)`/`INTEGER` over `BIGINT` below 19 digits, prefer `DOUBLE` over scaled `DECIMAL` where acceptable, prefer `VARCHAR` over `CHAR(n)`, never emit an empty string (the engine stores it as NULL), and input `TIMESTAMP` always arrives at microsecond precision. [F6]
- [ ] 4.3 Set `lto = "fat"` and `codegen-units = 1` in the workspace `[profile.release]` and in the scaffold template in `crates/cargo-exasol-udf/src/new.rs`, keeping `panic = "unwind"`. [F7]
- [ ] 4.4 Add a `native` benchmark shape to `benches/emit-bench` using only `DECIMAL(18,0)` and `DOUBLE` columns, and correct the `mixed` shape's description and `bytes_per_row` estimate, which both assume `id BIGINT` travels as a native integer. [F11]

## Parallelization

| Group | Tasks | Depends on | Knowledge |
|-------|-------|------------|-----------|
| A: emit/ingest codec and emit ownership | 1.1-1.10 | — | spec deltas `runtime/rowset-codec`, `runtime/dispatch-run-loop`, `runtime/emit-arrow-batch`, `sdk/udf-sdk`, `sdk/udf-abi`, `examples/test-udfs`; `crates/exa-udf-runtime/src/rowset.rs`, `crates/exa-udf-runtime/src/rowset_tests.rs`, `crates/exa-udf-runtime/src/dispatch.rs`, `crates/exasol-udf-sdk/src/context.rs`, `crates/exasol-udf-sdk/src/abi.rs`, `crates/exasol-udf-sdk/src/abi_tests.rs`, `test-udfs/emit-k/src/lib.rs`, `crates/exa-udf-runtime/tests/dispatch.rs`, `crates/exa-udf-runtime/tests/emit_arrow_dlopen.rs`, `crates/exa-udf-runtime/tests/ingest_alloc.rs`, `crates/it/tests/db_roundtrip.rs` |
| B: ZMQ transport frames | 2.1-2.4 | — | spec delta `protocol/handshake`; `crates/exa-zmq-protocol/src/transport.rs`, `crates/exa-zmq-protocol/src/transport_tests.rs`, `crates/exa-zmq-protocol/tests/transport.rs` |
| C: connect-back streaming and diagnostics | 3.1-3.3 | — | spec deltas `runtime/connect-back-query`, `runtime/debug-output`; `crates/exa-udf-runtime/src/connect_back.rs`, `crates/exa-udf-runtime/src/connect_back_tests.rs`, `crates/exa-udf-runtime/tests/debug_level.rs` |
| D: docs, build profile and benchmark | 4.1-4.4 | — | spec delta `tools/cargo-exaudf`; `docs/writing-a-udf.md`, `Cargo.toml`, `crates/cargo-exasol-udf/src/new.rs`, `crates/cargo-exasol-udf/tests/new.rs`, `benches/emit-bench/src/main.rs`, `benches/emit-bench/README.md` |

Group A is deliberately large. Findings F1, F3, F4, F5 and F8 all rewrite `crates/exa-udf-runtime/src/rowset.rs`, and F8 rewrites the borrow structure that hands that file's `EmitBuffer` to `dispatch.rs`. Splitting them would put two agents on one file and make each re-derive the same ownership model. The issue reaches the same conclusion for F3 and F4.

## Dead Code Removal

| Type | Location | Reason |
|------|----------|--------|
| Constant | `MAX_TOTAL_WAIT`, `crates/exa-zmq-protocol/src/transport.rs` | The retry loop no longer has a wall-clock deadline |
| Constant | `NUMERIC_COST_BASE`, `crates/exa-udf-runtime/src/rowset.rs` | Replaced by the exact rendered-decimal length |
| Function | `cb_log`, `crates/exa-udf-runtime/src/connect_back.rs` | Replaced by `tracing::debug!` |
| Method | `InputRowSet::row`, `crates/exa-udf-runtime/src/rowset.rs` | Replaced by forward-only `seek_row` |
| Field | `InputRowSet::rows: Vec<Vec<Value>>`, `crates/exa-udf-runtime/src/rowset.rs` | Replaced by the proto typed arrays plus one scratch row |
| Scenario | `runtime/connect-back-query` "RuntimeExaConnection streams query results as Value rows" | States the same requirement as the streaming scenario it duplicates |

## Verification

### Scenario Coverage

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| rowset-codec: Emit stamps the source input row number for pass-through columns | Integration | `crates/it/tests/db_roundtrip.rs` | `scalar_emits_passthrough_column_resolves_source_row` |
| rowset-codec: Emit stamps the source input row number for pass-through columns | Unit | `crates/exa-udf-runtime/src/rowset_tests.rs` | `to_proto_stamps_row_number_for_scalar_and_leaves_it_empty_for_set` |
| rowset-codec: InputRowSet materialises one row at a time from the proto blocks | Integration | `crates/exa-udf-runtime/tests/ingest_alloc.rs` | `from_proto_allocation_count_is_independent_of_row_count` |
| rowset-codec: InputRowSet materialises one row at a time from the proto blocks | Unit | `crates/exa-udf-runtime/src/rowset_tests.rs` | `input_rowset_round_trips_all_exatypes_after_storage_change` |
| rowset-codec: EmitBuffer tracks a running byte estimate and reports when to flush | Unit | `crates/exa-udf-runtime/src/rowset_tests.rs` | `numeric_byte_cost_equals_rendered_decimal_length` |
| rowset-codec: EmitBuffer tracks a running byte estimate and reports when to flush | Unit | `crates/exa-udf-runtime/src/rowset_tests.rs` | `group_reset_retains_capacity_and_leaves_flush_count` |
| rowset-codec: EmitBuffer packs output values row-major by declared column type | Unit | `crates/exa-udf-runtime/src/rowset_tests.rs` | `to_proto_consumes_rows_and_moves_strings` |
| handshake: Transient EAGAIN on recv/send is retried without a wall-clock cap | Unit | `crates/exa-zmq-protocol/src/transport_tests.rs` | `retry_transient_survives_ten_thousand_eagain_polls` |
| handshake: Transient EAGAIN on recv/send is retried without a wall-clock cap | Integration | `crates/exa-zmq-protocol/tests/transport.rs` | `recv_waits_through_a_reply_slower_than_the_poll_interval` |
| handshake: Transport round-trips a request and response over one frame each | Integration | `crates/exa-zmq-protocol/tests/transport.rs` | `transport_round_trip_single_frame` |
| dispatch-run-loop: Emit buffer and wire closures are session-scoped and reset per group | Integration | `crates/exa-udf-runtime/tests/dispatch.rs` | `session_scoped_emit_buffer_is_reused_across_groups` |
| emit-arrow-batch: push_batch produces proto blocks identical to the row-based push path | Unit | `crates/exa-udf-runtime/src/rowset_tests.rs` | `push_batch_matches_row_path_including_row_number` |
| emit-arrow-batch: push_batch produces proto blocks identical to the row-based push path | Integration | `crates/exa-udf-runtime/tests/emit_arrow_dlopen.rs` | `emit_arrow_dlopen_round_trips_batch` |
| udf-sdk: UdfContext exposes typed accessors and row iteration | Integration | `crates/exasol-udf-sdk/tests/emit_owned.rs` | `emit_owned_default_forwards_to_emit` |
| test-udfs: emit-k emits a variable number of rows per input row | Integration | `crates/it/tests/db_roundtrip.rs` | `emit_k_scalar_emits_zero_one_many` |
| udf-abi: Owned-row emit widens the UdfContext vtable and bumps the ABI version | Unit | `crates/exasol-udf-sdk/src/abi_tests.rs` | `abi_version_is_eight` |
| udf-abi: Owned-row emit widens the UdfContext vtable and bumps the ABI version | Integration | `crates/exa-udf-runtime/tests/loader.rs` | `loader_rejects_mismatched_abi_version` |
| connect-back-query: query_for_each streams the result set one batch at a time | Integration | `crates/it/tests/db_roundtrip.rs` | `connect_back_stream_reads_all_rows` |
| debug-output: Connect-back diagnostics use the gated tracing channel and write no file | Integration | `crates/exa-udf-runtime/tests/debug_level.rs` | `connect_back_diagnostics_are_gated_and_write_no_file` |
| cargo-exaudf: new scaffolds a buildable UDF crate | Integration | `crates/cargo-exasol-udf/tests/new.rs` | `new_scaffolds_crate_files` |

The zero-copy properties of tasks 2.2 and 2.3 are not observable from a unit assertion. Task 4.4's benchmark measures them, and the round-trip test above proves the behaviour is unchanged.

### Manual Testing

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| runtime/rowset-codec (F1) | `cargo test -p it --features integration -- --nocapture` then read the `scalar_emits_passthrough_column_resolves_source_row` line | Scenario reports ok; every pass-through value equals its source input |
| protocol/handshake (F2) | `rtk proxy grep -rn MAX_TOTAL_WAIT crates/` | No match |
| protocol/handshake (F3, F4) | `cargo test -p exa-zmq-protocol` | 0 failures |
| runtime/dispatch-run-loop (F8) | `cargo test -p exa-udf-runtime --all-features` | 0 failures |
| sdk/udf-abi | `cargo exasol-udf validate target/release/libemit_k.so` | Reports abi_version 8 and a matching fingerprint |
| runtime/connect-back-query (F9) | `cargo test -p it --features integration` then read the `connect_back_stream` line | Scenario reports ok |
| runtime/debug-output (F10) | Run the IT suite, then `ls /tmp/cb_debug.txt` | `No such file or directory` |
| tools/cargo-exaudf (F7) | `cargo exasol-udf new /tmp/perf-udf && rtk proxy grep -n lto /tmp/perf-udf/Cargo.toml` | `lto = "fat"` and `codegen-units = 1` present |
| docs (F6) | `rtk proxy grep -n "INTEGER" docs/writing-a-udf.md` | States `INTEGER` arrives as `Value::Int64` |
| benches/emit-bench (F11) | `cargo run --release -p emit-bench` | Result table lists a `native` shape row alongside `mixed` and `wide` |

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Build | `cargo build --release` | Exit 0 |
| Unit + integration (host) | `cargo test` | 0 failures |
| All features | `cargo test --all-features` | 0 failures |
| Live DB integration | `cargo test -p it --features integration` | 0 failures |
| Lint | `cargo clippy --all-targets --all-features -- -D warnings` | 0 warnings |
| Format | `cargo fmt --check` | No changes |
</content>
