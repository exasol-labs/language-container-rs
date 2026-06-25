# Plan: add-emit-batch-arrow

## Summary

Add an opt-in `ctx.emit_batch(&RecordBatch)` API to `UdfContext` so UDF authors whose data is already in Arrow form can emit Arrow record batches directly, with the host encoding Arrow columns **vectorised, column-at-a-time** straight into the columnar proto type blocks under the existing 4,000,000-byte `MT_EMIT` flush semantics. Because that limit is a hard cap and a batch may exceed it, the encoder splits the batch at row-granular boundaries (computed from a cheap cumulative per-row byte cost) and flushes each ≤4 MB zero-copy `RecordBatch::slice` directly, materialising only the sub-4 MB tail. The capability lives behind a new standalone `emit-arrow` SDK feature that `connect-back` implies.

## Design

### Context

UDF authors who already produce data as Arrow `RecordBatch`es (the natural output of an Arrow-based source or connect-back reads) must currently transpose every batch into `Vec<Value>` rows and call `ctx.emit` per row before the host transposes those rows *back* into the columnar proto type blocks. That is two redundant pivots over the same data, with a per-cell `downcast_ref` and a per-cell `Value` allocation. We want a direct columnar emit path — vectorised, one downcast per column and one bulk null-buffer read — while leaving the row-based `emit` and all flush/threshold behaviour untouched.

- **Goals**
  - A defaulted `UdfContext::emit_batch(&RecordBatch)` UDF authors can call alongside row-based `emit`, with no breaking change to existing `UdfContext` implementations.
  - Host-side direct Arrow-column → proto-type-block encoding (no intermediate `Vec<Value>` per row).
  - Reuse the existing `EmitBuffer` byte-estimate accounting and `4_000_000`-byte flush threshold verbatim — batches flush exactly as rows do.
  - Make Arrow batch emit usable WITHOUT enabling connect-back (a new `emit-arrow` feature), since not every batch-emitting UDF connects back.
- **Non-Goals**
  - Deriving the output schema from the Arrow batch. The declared EMITS `ColumnMeta` remains the authority for the target `ExaType` of each column (see Decision below).
  - Any change to the wire protocol, proto schema, or the 2 GB per-value ceiling.
  - Putting Arrow on the wire or changing the proto schema. The DB only decodes columnar `ExascriptTableData`; the batch is converted host-side. A batch larger than 4 MB is split at row-granular boundaries (the only granularity the wire format allows) and flushed across several `MT_EMIT` frames — `push_batch` cannot flush strictly at batch boundaries because the 4 MB cap is hard and batch size is unknown at compile time.

### Decision

`ColumnMeta` (the declared EMITS schema), not the Arrow schema, dictates which proto block each column lands in. Research into `exarrow-rs` (`docs/type-mapping.md`) confirms the Arrow `DataType` is *not* sufficient to recover the Exasol type: `Utf8` collapses VARCHAR/CHAR/GEOMETRY/HASHTYPE/INTERVAL, and several Exasol types share one Arrow type. The host therefore dispatches on the Arrow `DataType` only to *extract* each cell value, then packs it into the block named by the column's declared `ExaType` — mirroring the existing `EmitBuffer::to_proto` row path exactly.

#### Architecture

```
UDF run()                          exa-udf-runtime (host)
  build RecordBatch                  HostContextBridge::emit_batch(&batch)
  ctx.emit_batch(&batch) ──ABI──▶      └─ emit_buf.push_batch(&batch, output_meta)
                                          ├─ cumulative per-row byte cost (offset buffer / fixed widths)
                                          ├─ split into ≤4 MB row ranges
                                          └─ per slice: zero-copy RecordBatch::slice
                                               └─ per column (downcast once, bulk nulls) ──▶ proto block
                                                                  (block chosen by declared ExaType)
                                          └─ flush each full slice → MT_EMIT; tail (<4 MB) → shared buffer
  (run returns) ───────────────────▶ dispatch tail flush → final MT_EMIT
```

The `RecordBatch` is a host-owned arrow type; it crosses the `.so` boundary only as `&RecordBatch` and is consumed entirely host-side, so no Arrow `TypeId` is downcast in UDF code (the same FFI discipline `query_arrow` documents).

#### Patterns

| Pattern | Where | Why |
|---------|-------|-----|
| Defaulted trait method returning `Unimplemented` | `UdfContext::emit_batch` | Backward-compatible extension; mirrors `memory_limit`, `connection`, etc. |
| Declared-metadata-authoritative encoding | `EmitBuffer::push_batch` | Arrow `DataType` is ambiguous for Exasol types; the EMITS `ColumnMeta` is the single source of truth, identical to `to_proto` |
| Vectorised column-at-a-time extraction | `EmitBuffer::push_batch` | One `downcast_ref` per column + one bulk null-buffer read replaces per-cell downcast/alloc; row-major-interleaved block layout preserved so it stays byte-identical to `to_proto` |
| Row-granular 4 MB split via offset prefix-sum | `EmitBuffer::push_batch` | Hard `MT_EMIT` cap + unknown batch size forces splits; offset buffer gives per-row variable-width cost without touching data bytes; zero-copy `RecordBatch::slice` per range |
| Feature implication | `connect-back = [… "emit-arrow"]` | Connect-back UDFs are the primary batch-emit consumers and already link `arrow` |
| Shared byte accounting | `push_batch` matches `value_byte_cost`/`byte_estimate` | One flush rule for both emit styles |

### Consequences

| Decision | Alternatives Considered | Rationale |
|----------|------------------------|-----------|
| `push_batch` encodes vectorised column-at-a-time, splitting at row-granular 4 MB boundaries via the offset-buffer prefix-sum and flushing each zero-copy slice | (a) Convert batch → `Vec<Value>` then `push` per row; (b) flush strictly at batch boundaries; (c) `extend` whole columns into blocks | (a) is the double pivot the feature exists to remove; (b) is incorrect — a batch can exceed the hard 4 MB cap; (c) scrambles the row-major-interleaved block layout when columns share a block type or nulls exist. Per-column downcast + bulk null read is the fastest *correct* path |
| Representation: keep `Vec<Value>` buffer; flush pending rows, encode full slices directly, materialise only the <4 MB tail | Refactor `EmitBuffer` to accumulate proto blocks incrementally (no `Vec<Value>`) | Smallest correct change: row `emit`/`to_proto` and "one buffer, one tail flush" untouched; double pivot avoided for all but the sub-4 MB tail, which cannot exceed the buffer's existing ceiling (decision-log [7]) |
| Output `ExaType` (declared `ColumnMeta`) chooses the target block | Derive Exasol type from Arrow `DataType` | exarrow-rs mapping proves Arrow `DataType` is ambiguous (`Utf8`→5 Exasol types); declared schema is authoritative, matching `to_proto` |
| New standalone `emit-arrow` feature; `connect-back` implies it | Gate `emit_batch` under `connect-back` | Batch emit is useful without connect-back (pure compute UDFs); keeping it separate avoids forcing tokio/exarrow-rs/rustls onto non-connecting UDFs |
| Thread output `ColumnMeta` into `HostContextBridge` | Re-derive output meta inside `emit_batch` | The bridge currently holds only input columns; the output meta lives only in the flusher closure. `emit_batch` needs it, so dispatch must pass it into the constructor |

## Features

| Feature | Status | Spec |
|---------|--------|------|
| sdk/udf-sdk | CHANGED | `specs/_plans/add-emit-batch-arrow/sdk/udf-sdk/spec.md` |
| runtime/dispatch-run-loop | CHANGED | `specs/_plans/add-emit-batch-arrow/runtime/dispatch-run-loop/spec.md` |
| examples/test-udfs | CHANGED | `specs/_plans/add-emit-batch-arrow/examples/test-udfs/spec.md` |

## Dependencies

- `arrow = "58"` (already a workspace dependency, pinned to match `exarrow-rs`). The `emit-arrow` feature makes it optional-but-activatable independently of `connect-back`. **Unchanged by this plan** — exarrow-rs 0.13.0 still requires `arrow ^58`, so the bump below is not an arrow bump and does not touch the encoder design.
- `exarrow-rs` bumped `^0.12.8` → `^0.13.0` (latest, released 2026-06-24). 0.13.0's breaking removals (`connection::session::SessionManager`, the never-constructed `ExportError::{CsvParse,Arrow,Schema,Parquet}` variants) are not in our usage surface — the host (`exa-udf-runtime/src/connect_back.rs`) and IT crate use only `exarrow_rs::Parameter`, `exarrow_rs::adbc::{Connection, Driver}`, and `exarrow_rs::error::QueryError`, all unchanged — so the bump is expected to be a clean recompile with no source edits. See decision-log [8].

## Implementation Tasks

0. **Dependency bump (independent; can run first)**
   - [ ] 0.1 Bump `exarrow-rs` to `^0.13.0` in the workspace `Cargo.toml`; run `cargo update -p exarrow-rs` and confirm `Cargo.lock` resolves exarrow-rs 0.13.0 **and arrow stays at 58** (no arrow version drift).
   - [ ] 0.2 Verify a clean build/test with the bump and no source changes: `cargo build --release`, `cargo test`, `cargo clippy --all-targets --all-features -- -D warnings`, and the live-DB `cargo test -p it --features integration` (exarrow-rs is the connect-back transport, so the integration suite is the real gate). If any of our used APIs did change, fix the call sites — but per decision-log [8] none are expected.

1. **SDK feature plumbing**
   - [ ] 1.1 Add `emit-arrow = ["dep:arrow"]` to `crates/exasol-udf-sdk/Cargo.toml`; add `emit-arrow` to the `connect-back` feature list.
   - [ ] 1.2 In `crates/exasol-udf-sdk/src/context.rs`, add the defaulted `emit_batch(&mut self, batch: &arrow::record_batch::RecordBatch) -> Result<(), UdfError>` method gated `#[cfg(feature = "emit-arrow")]`, default returning `UdfError::Unimplemented("emit_batch".into())`.
   - [ ] 1.3 Add a unit test in `context.rs` asserting the default `emit_batch` returns `Unimplemented` (compiled under `emit-arrow`).

2. **Runtime: direct Arrow encoding**
   - [ ] 2.1 Add `emit-arrow` to `crates/exa-udf-runtime/Cargo.toml` features: `emit-arrow = ["exasol-udf-sdk/emit-arrow", "dep:arrow"]`; ensure `connect-back` activates `emit-arrow` (and the runtime's `arrow`) too.
   - [ ] 2.2 Implement `EmitBuffer::push_batch(&mut self, batch: &RecordBatch, meta: &[ColumnMeta], flusher: …) -> Result<(), UdfError>` in `crates/exa-udf-runtime/src/rowset.rs`, gated `#[cfg(feature = "emit-arrow")]`, with the vectorised column-at-a-time encoder: (1) compute a cumulative per-row byte cost across the batch with NO per-cell work — fixed-width Arrow types (`Int32`/`Int64`/`Float64`/`Boolean`/`Date32`) by width, variable-width (`Utf8`/`LargeUtf8`) via a prefix-sum over the Arrow offset buffer (never touching the data buffer), `Decimal128`/`Timestamp` via the same fixed estimate as `value_byte_cost` (`Numeric` = `40 + scale`, `Timestamp` = `29`, `Date` = `10`), NULL = 0; (2) including any bytes already buffered, find row-range split points where the running total reaches `EMIT_BUFFER_LIMIT_BYTES` (`4_000_000`) — if pending Value rows are buffered, flush them first so the batch starts from empty; (3) for each full ≤4 MB range take a zero-copy `RecordBatch::slice(offset, len)`, encode its columns vectorised (downcast each array ONCE, read its null buffer ONCE in bulk) into one `ExascriptTableData` preserving the dense row-major-interleaved block layout `to_proto` produces, and flush it; (4) materialise the trailing <4 MB remainder into the shared `rows` buffer (the one bounded tail pivot). Return `Err(UdfError)` when a column's Arrow `DataType` cannot feed the declared `ExaType` block. [expert]
   - [ ] 2.3 Add `output_meta: &'a [ColumnMeta]` field to `HostContextBridge`; thread it through `new` and `with_connection`; update `dispatch.rs` `run_batch` to pass `&meta.output_columns`. [expert]
   - [ ] 2.4 Implement `HostContextBridge::emit_batch` override gated `#[cfg(feature = "emit-arrow")]`: call `push_batch(batch, self.output_meta, &mut flush_slice)` passing the bridge's existing flusher as the per-full-slice flush callback so each ≤4 MB slice is sent mid-batch; after `push_batch` returns (only the <4 MB tail remains buffered) run the same `should_flush()`/flush tail check the row-based `emit` uses. The end-of-`run` dispatch tail flush still sends whatever remains — one buffer, one tail flush.
   - [ ] 2.5 Unit tests in `rowset.rs`, gated `#[cfg(feature = "emit-arrow")]`: (a) a batch whose cumulative cost exceeds `4_000_000` splits into multiple flushes at the correct row boundaries (assert the flush callback fires N times and each flushed `ExascriptTableData.rows` sums to the batch row count, with each flushed slice's estimate ≤ 4 MB); (b) parity — the `push_batch` path serialised via `to_proto` is byte-identical to the row-based `push`/`to_proto` for the same data, including a schema with multiple columns sharing one block type (exercising the row-major-interleaved layout); (c) null-bitmap correctness driven by the Arrow validity buffer (NULL cells occupy no type-block slot, only the row-major bitmap); (d) the cumulative byte-cost estimate matches the row path's `value_byte_cost` accounting closely enough that `should_flush` fires at the same threshold for the same data; (e) a column whose Arrow `DataType` cannot feed the declared `ExaType` returns `Err`; (f) `RecordBatch::slice` is zero-copy (the trailing tail materialisation never exceeds 4 MB). [expert]

3. **Example test UDF**
   - [ ] 3.1 Scaffold `test-udfs/emit-arrow-batch/` (Cargo.toml with `exasol-udf-sdk` features = `["emit-arrow"]`, `exasol-udf-macros`, `arrow`; `crate-type = ["cdylib"]`).
   - [ ] 3.2 Implement `src/lib.rs`: drain input, build a `RecordBatch` (Int64 + Utf8 columns), call `ctx.emit_batch(&batch)` once. Add to the workspace members if needed.

4. **Integration coverage**
   - [ ] 4.1 Add a `db_roundtrip.rs` scenario `emit_arrow_batch_roundtrips` in `crates/it`: register the `emit-arrow-batch` UDF as a RUST SET SCRIPT, invoke it, assert the emitted rows match the batch the UDF built.
   - [ ] 4.2 Wire the new `.so` into the IT harness upload list (mirror `EMIT_BULK_LIB`).

5. **Docs (minor)**
   - [ ] 5.1 Note `emit_batch` and the `emit-arrow` feature in `docs/writing-a-udf` (or the relevant SDK doc page).

## Parallelization

| Parallel Group | Tasks |
|----------------|-------|
| Group 0 | 0.1, 0.2 (exarrow-rs bump — independent; arrow unchanged, so does not block any emit-batch work) |
| Group A | 1.1, 1.2, 1.3 (SDK) |
| Group B | 3.1, 3.2 (example crate — depends on Group A) |
| Group C | 2.1, 2.2, 2.3, 2.4, 2.5 (runtime — depends on Group A) |
| Group D | 4.1, 4.2, 5.1 (IT + docs — depend on B and C) |

Sequential dependencies:
- Group 0 is independent (runs first or alongside anything; the arrow pin is unchanged)
- Group A → Group B, Group C
- Group B, Group C → Group D

## Dead Code Removal

| Type | Location | Reason |
|------|----------|--------|
| (none) | — | Purely additive; no existing code is replaced or obsoleted |

## Verification

### Scenario Coverage

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| emit-arrow feature pulls in arrow independently of connect-back | Integration | `crates/exasol-udf-sdk/Cargo.toml` + CI build matrix | `cargo build -p exasol-udf-sdk --features emit-arrow` (compiles; `--no-default-features` excludes arrow) |
| UdfContext exposes emit_batch as a defaulted method behind emit-arrow | Unit | `crates/exasol-udf-sdk/src/context.rs` | `default_emit_batch_unimplemented` |
| EmitBuffer encodes an Arrow batch column-at-a-time into proto type blocks | Unit | `crates/exa-udf-runtime/src/rowset.rs` | `push_batch_null_bitmap`, `push_batch_byte_estimate_parity`, `push_batch_type_mismatch_errors` |
| push_batch splits an oversized batch at row boundaries under the 4 MB cap | Unit | `crates/exa-udf-runtime/src/rowset.rs` | `push_batch_splits_oversized_batch`, `push_batch_slice_zero_copy_tail_bounded` |
| push_batch produces proto blocks identical to the row-based push path | Unit | `crates/exa-udf-runtime/src/rowset.rs` | `push_batch_equals_row_push`, `push_batch_shared_block_type_interleaved` |
| Bridge carries the output metadata and flushes emit_batch on the same threshold | Unit | `crates/exa-udf-runtime/src/rowset.rs` | `bridge_emit_batch_buffers_and_flushes`, `bridge_emit_batch_error_propagates` |
| emit and emit_batch share one buffer and one tail flush | Unit | `crates/exa-udf-runtime/src/rowset.rs` | `bridge_mixed_emit_styles_share_buffer` |
| emit-arrow-batch emits a manually built Arrow RecordBatch | Integration | `crates/it/tests/db_roundtrip.rs` | `emit_arrow_batch_roundtrips` |

### Manual Testing

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| sdk/udf-sdk | `cargo build -p exasol-udf-sdk --features emit-arrow` then `cargo build -p exasol-udf-sdk --no-default-features` | First links arrow; second compiles with no arrow dep |
| examples/test-udfs | `cargo exasol-udf build` in `test-udfs/emit-arrow-batch` | `target/x86_64-unknown-linux-musl/release/libemit_arrow_batch.so` produced |
| runtime/dispatch-run-loop | `cargo test -p exa-udf-runtime --features emit-arrow push_batch` | Round-trip and parity unit tests pass |
| examples/test-udfs (e2e) | `cargo test -p it --features integration emit_arrow_batch` | Emitted rows equal the UDF-built batch |

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Build | `cargo build --release` | Exit 0 |
| Build (feature) | `cargo build -p exa-udf-runtime --features emit-arrow` | Exit 0 |
| Test | `cargo test` | 0 failures |
| Integration | `cargo test -p it --features integration` | 0 failures (live Docker DB) |
| Lint | `cargo clippy --all-targets --all-features -- -D warnings` | 0 warnings |
| Format | `cargo fmt --check` | No changes |
