# Decision Log: add-emit-batch-arrow

Date: 2026-06-24

## Interview

**Q:** From which side of the `.so` boundary is `emit_batch` called?
**A:** UDF-side. The UDF author chooses between `ctx.emit(&[Value])` (row style) and `ctx.emit_batch(&RecordBatch)` (Arrow style). Both are methods on `UdfContext`.

**Q:** What is the output column metadata source for encoding the Arrow batch?
**A:** The Arrow schema must be translated into Exasol data types. The architect asked to check `exarrow-rs` (exasol-labs/exarrow-rs) for how they map Arrow ↔ Exasol types, to decide whether the Arrow `DataType` alone is enough or the declared `ColumnMeta` is always needed.

**Q:** Should this be feature-gated, and how does it relate to `connect-back`?
**A:** Yes — a new `emit-arrow` feature in `exasol-udf-sdk`, standalone (not `connect-back`). The `connect-back` feature implies `emit-arrow` in `Cargo.toml`, since Arrow-producing connect-back UDFs are the primary consumer. The `arrow` dependency moves from `connect-back`-only to also being pulled in by `emit-arrow`.

**Q:** What is the encoding path — through `Vec<Value>` or direct?
**A:** Direct Arrow → `EmitBuffer`, no `Vec<Value>` intermediate. A new `EmitBuffer::push_batch(&RecordBatch, &[ColumnMeta])` reads Arrow columns directly into the proto type blocks (double_data, int32_data, string_data, …). Existing `push` and byte-estimate accounting are unchanged; `push_batch` uses the same accounting.

## Design Decisions

### [1] Declared EMITS `ColumnMeta` (not Arrow schema) is authoritative for the target proto block

- **Decision:** `push_batch` dispatches on the Arrow `DataType` only to extract each cell value, then packs it into the proto block dictated by the declared output `ExaType` — identical to the existing `to_proto` row path.
- **Alternatives:** Derive the Exasol type purely from the Arrow `DataType` (no `ColumnMeta` needed).
- **Rationale:** exarrow-rs `docs/type-mapping.md` confirms the Arrow `DataType` is ambiguous for Exasol: `Utf8` maps to VARCHAR/CHAR/GEOMETRY/HASHTYPE/INTERVAL, and multiple Exasol types collapse onto one Arrow type. Only the declared EMITS schema can name the correct block. This also keeps `push_batch` byte-identical to the row-based path.
- **Promotes to ADR:** yes

### [2] New standalone `emit-arrow` feature; `connect-back` implies it

- **Decision:** Add `emit-arrow = ["dep:arrow"]` to `exasol-udf-sdk`; add `emit-arrow` to the `connect-back` feature list so connect-back continues to pull in arrow transitively.
- **Alternatives:** Gate `emit_batch` directly under `connect-back`.
- **Rationale:** Pure-compute UDFs (an Arrow-based source without connect-back) want batch emit without dragging in tokio/exarrow-rs/rustls. A separate feature keeps the dependency surface minimal while connect-back UDFs — the primary consumers — get it for free.
- **Promotes to ADR:** yes

### [3] Direct Arrow-column → proto-block encoding, no `Vec<Value>` intermediate

- **Decision:** `EmitBuffer::push_batch` encodes Arrow arrays straight into the proto type-block vectors, reusing the existing `byte_estimate` accounting and the `4_000_000`-byte flush threshold.
- **Alternatives:** Reuse `record_batch_to_rows` to produce `Vec<Value>` rows, then `push` each row.
- **Rationale:** The feature exists precisely to remove the redundant double pivot (Arrow→Value→proto). Direct encoding is the whole point; the row-conversion helper remains for connect-back reads where rows must cross the FFI boundary.
- **Promotes to ADR:** no

### [6] Vectorised column-at-a-time extraction with row-granular 4 MB split points

- **Decision:** `push_batch` encodes the batch **column-at-a-time (vectorised)**, not cell-by-cell. Each Arrow column array is downcast to its concrete type **once** (not once per row, as `record_batch_to_rows`/`cell_to_value` do) and its validity (null) buffer is read once in bulk. Because the `MT_EMIT` wire limit `EMIT_BUFFER_LIMIT_BYTES = 4_000_000` is a hard cap and the batch size is unknown at compile time (a batch may exceed 4 MB), the encoder cannot flush strictly at batch boundaries. It instead:
  1. Computes a cheap **cumulative per-row byte cost** across the batch with **no per-cell work**: fixed-width Arrow types (`Int32`/`Int64`/`Float64`/`Boolean`/`Date32`) cost `width` per row; variable-width (`Utf8`/`LargeUtf8`) per-row costs come from the Arrow **offset buffer** (consecutive offset deltas) — a prefix-sum over offsets, never touching the data buffer; `Decimal128`→string and `Timestamp`→string use the same fixed rendered-length estimate the row path's `value_byte_cost` uses (`Numeric` = `40 + scale`, `Timestamp` = `29`, `Date` = `10`), so the batch estimate matches the row-path accounting and `should_flush` fires at the same threshold. A NULL cell costs 0, identical to `value_byte_cost`.
  2. Walks the cumulative costs (plus any bytes already buffered from a prior `push`/`push_batch`) to find **row-range split points** where the running total reaches 4 MB.
  3. For each ≤4 MB row range takes a **zero-copy `RecordBatch::slice(offset, len)`** (Arrow slices share the underlying buffers, no data copy), encodes that slice's columns vectorised into one `ExascriptTableData`, and flushes it.
  4. Leaves the trailing partial (<4 MB) range buffered for a later `emit`/`push_batch` or the end-of-`run` tail flush.
- **Constraint that shapes the encoding order:** the proto type blocks are **dense and row-major interleaved** — `to_proto` fills them by iterating rows then columns, NULLs occupy no slot, and `from_proto` decodes by advancing one cursor per block type across rows in that same order. A naïve "`extend` the whole Arrow column into its block" is therefore byte-identical to the row path **only** when at most one column maps to a given block type and no nulls are present. With multiple same-`ExaType` columns or any nulls the block ordering is data-dependent. So vectorisation lives in the **per-column typed downcast + bulk null-buffer read** (done once per column, not per cell), feeding a row-major block assembly that preserves the interleaved dense layout `from_proto` expects. The win is eliminating the per-cell `as_any().downcast_ref` and per-cell `Value` allocation, not changing the block layout.
- **Alternatives:** (a) Flush strictly at batch boundaries — rejected: a batch can exceed the hard 4 MB `MT_EMIT` cap, so this is incorrect, not merely slow. (b) Cell-by-cell extraction (downcast per cell) — rejected: that is the per-row pivot the feature exists to remove. (c) True bulk `extend` of whole columns into blocks — rejected as the general path because it scrambles the row-major-interleaved layout whenever multiple columns share a block type or nulls are present; correctness must hold for arbitrary EMITS schemas.
- **Rationale:** Arrow's columnar layout maps 1:1 onto the columnar proto blocks, so extracting per-column with a single downcast and a single null-buffer scan is the fastest correct strategy, while the offset-buffer prefix-sum gives 4 MB split points without ever materialising a `Vec<Value>` or touching variable-width data bytes. The hard wire cap forces row-granular splits regardless of vectorisation.
- **Promotes to ADR:** yes

### [7] EmitBuffer representation under the batch path: flush-pending-then-encode-slices (Option A)

- **Decision:** Keep `EmitBuffer`'s existing `rows: Vec<Vec<Value>>` + lazy `to_proto(meta)` representation unchanged. On `push_batch`: (1) if Value rows are already buffered, flush them first so the batch starts from an empty buffer; (2) split the batch into ≤4 MB zero-copy slices and encode/flush each full slice directly to `ExascriptTableData` (bypassing the `Vec<Value>` buffer entirely for the bulk of the data); (3) materialise **only** the trailing <4 MB remainder slice into the shared `rows` buffer (the one acceptable, bounded, one-time pivot — of the tail only), so subsequent `emit` calls and the end-of-`run` tail flush stay coherent with a single buffer and a single flush path.
- **Why the tail pivot cannot exceed 4 MB and is acceptable:** by construction step (2) peels off every full ≤4 MB slice; the remainder is the bytes left after the last split point, which is strictly below `EMIT_BUFFER_LIMIT_BYTES`. Pivoting it to `Vec<Value>` is therefore bounded by the same 4 MB the row path already buffers, so it adds no new memory ceiling and runs once per `push_batch`, not per row.
- **Alternatives:** Option B — refactor `EmitBuffer` to accumulate proto blocks incrementally (both `emit` and `push_batch` append into the same in-progress `ExascriptTableData`), eliminating `Vec<Value>` retention entirely.
- **Rationale (ponytail / smallest correct change):** Option A is the minimal change that meets the performance goal: the redundant Arrow→Value→proto double pivot is avoided for all but the sub-4 MB tail, the existing row `emit`/`to_proto` path and all flush semantics are untouched (no regression surface), and "one buffer, one tail flush" is preserved verbatim. Option B is more performant in the pathological case of many tiny interleaved `emit`/`emit_batch` calls and fully columnar, but it rewrites the row path, `to_proto`, and `from_proto`-symmetry guarantees — a large regression surface for a marginal gain over a tail that is already capped at 4 MB. Reject B unless profiling later shows the tail pivot dominates.
- **Promotes to ADR:** no

### [4] `emit_batch` is a defaulted `UdfContext` method returning `Unimplemented`

- **Decision:** `emit_batch` is a provided trait method (default `Err(UdfError::Unimplemented("emit_batch"))`), gated `#[cfg(feature = "emit-arrow")]`; row-based `emit` stays a required method.
- **Alternatives:** Make it a required method (breaks every existing `UdfContext` impl) or a separate trait.
- **Rationale:** Backward compatibility — mirrors the established SDK pattern (`memory_limit`, connect-back methods). Existing context impls (`SingleCallContext`, test mocks) keep compiling unchanged; only `HostContextBridge` overrides it.
- **Promotes to ADR:** no

### [5] Thread output `ColumnMeta` into `HostContextBridge`

- **Decision:** Add an `output_meta: &'a [ColumnMeta]` field to `HostContextBridge`, passed by `dispatch::run_batch` (the same `meta.output_columns` the flusher closure already serialises with).
- **Alternatives:** Re-derive or pass output meta on each `emit_batch` call.
- **Rationale:** The bridge currently holds only `input_cols`; the output schema lives solely inside the flusher closure. `emit_batch` needs it at the encoding site, so the cleanest path is to give the bridge the slice once at construction.
- **Promotes to ADR:** no

### [8] Bump exarrow-rs 0.12.8 → 0.13.0 (arrow unchanged at 58)

- **Decision:** Bump the workspace `exarrow-rs` pin `^0.12.8` → `^0.13.0` (latest, released 2026-06-24). The `arrow` pin stays at `^58`.
- **Why arrow is unaffected:** exarrow-rs 0.13.0 still depends on `arrow ^58`, so the workspace arrow version, the single-shared-copy guarantee, and the entire `emit-arrow` encoder design are untouched. This is a transport-library bump, not an arrow bump.
- **Why it is safe:** 0.13.0's breaking change removes only unused public API — `connection::session::SessionManager` and the never-constructed `ExportError::{CsvParse,Arrow,Schema,Parquet}` variants (plus the `From<parquet::errors::ParquetError>` impl). Our usage surface is only `exarrow_rs::Parameter`, `exarrow_rs::adbc::{Connection, Driver}`, and `exarrow_rs::error::QueryError` (in `crates/exa-udf-runtime/src/connect_back.rs` and `crates/it/`), none of which are removed. Expected to be a clean recompile with no source edits; still gated on the live-DB integration suite since exarrow-rs is the connect-back transport.
- **Optional follow-up (out of scope):** 0.13.0 adds `Connection::builder().validate_server_certificate(bool)`, mirroring the `validateservercertificate=0` DSN rule used for the self-signed Docker cert. The IT harness *could* later adopt the builder method instead of the DSN flag, but it is not required by this plan.
- **Promotes to ADR:** no

## Review Findings

- **No correctness bugs.** `encode_slice` is byte-identical to `to_proto` (row-major-interleaved layout, NULL bitmap, declared-`ExaType`-chooses-block); the 4 MB split fires at the same `>=` threshold as the row path; a single oversized row flushes alone; the tail is `< 4 MB` by construction; `validate_arrow_type` / `encode_slice` / `arrow_batch_to_value_rows` / `compute_row_costs` are mutually consistent (including the Int32/Int64/Float64 → `ExaType::Numeric` widenings for the BIGINT case, which arrives as `PB_NUMERIC`).
- **Spec-compliance fix (applied):** the first cut of `encode_slice`/`arrow_batch_to_value_rows` downcast per cell inside the `for r { for c }` loop, not once per column — violating the "column-at-a-time, downcast ONCE, read null buffer ONCE in bulk" MUST of the runtime spec scenario (decision [6]). Refactored to build per-column accessors (one downcast + one null-buffer capture per column) before the row-major loop, with a single `validate_arrow_type` site. Stale private-fn inline comments removed (kept the non-obvious Arrow-epoch arithmetic comment).
- **Version-bump note (not a code defect):** bumping the workspace `0.16.0 → 0.17.0` changes the `sdk_fingerprint` baked into every `.so`, so the two `exa-udf-runtime/tests/dispatch.rs` tests fail against stale `target/debug/*.so` until the fixture crates are rebuilt; they pass on rebuild (CI and the IT build rebuild all fixtures).

## E2E Outcome — BLOCKING DESIGN FLAW (confirmed)

The live-DB E2E (`scripts/ci-it-local.sh`, Exasol 2026.1.0) passes every existing scenario but the
new `emit_arrow_batch` scenario crashes the DB VM (`Internal error: VM crashed`, SQL state 22002) — a
hard SIGSEGV with no `F-UDF-CL-RUST` message and empty script-container cored logs (a catchable Rust
panic would have produced a clean error and been caught by the run-shim `catch_unwind`).

**Confirmed root cause (decisive in-process repro, no DB):** a no-DB driver dlopened the host-built
`libemit_arrow_batch.so` and called `ctx.emit_batch(&batch)` against a `HostContextBridge`. It
**SIGSEGV'd** — with BOTH the release and the debug `.so`, i.e. with the host runtime and the `.so`
sharing the *same* `arrow` source + rustc. Meanwhile `EmitBuffer::push_batch` on a **host-built**
`RecordBatch` passes all unit tests. The only difference is batch *provenance*: a `RecordBatch` built
inside the UDF `.so` and read by the host segfaults. Arrow's `Arc<dyn Array>` vtables and `Any`/`TypeId`
downcasting do not survive two independently linked static copies of `arrow` across a `dlopen`
boundary — exactly the invariant CLAUDE.md/mission.md state: *"Arrow types never cross the `.so`
boundary; Arrow TypeId is not stable across dynamic library boundaries."*

**Implication for the design:** decision [3] / the architecture diagram (`ctx.emit_batch(&batch) ──ABI──▶`
host reads/downcasts the arrays) is unsound. Passing `&RecordBatch` across the boundary cannot be made
safe, so the "host-side direct columnar encode with no `Vec<Value>` intermediate, RecordBatch crosses
only as `&RecordBatch`" mechanism must change.

**Required redesign (keeps Arrow off the boundary; the workspace `arrow` `ipc` feature is the enabler):**
serialize the `RecordBatch` to Arrow IPC bytes **inside the UDF `.so`** (via a blanket extension trait so
the serialization is monomorphised in the caller crate, not the host vtable), cross the boundary as
`&[u8]` through a new Arrow-free ABI method (e.g. `emit_record_batch_ipc(&[u8])`), and have the host
deserialize into its **own** `RecordBatch` and run the existing `push_batch` encoder unchanged. Cost:
one IPC serialize (UDF side) + one deserialize (host side) vs. the zero-copy cross that is not viable.
This is a spec/ABI change to `sdk/udf-sdk` and `runtime/dispatch-run-loop` and needs architect sign-off
before implementing. Pipeline stopped at the failed gate: NOT recorded, NO PR.
