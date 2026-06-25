# Tasks: add-emit-batch-arrow

## Phase 2: Implementation (Group 0 — exarrow-rs bump)
- [x] 0.1 Bump exarrow-rs `^0.12.8` → `^0.13.0`; cargo update; confirm arrow stays at 58
- [x] 0.2 Verify clean build/test/clippy with the bump (integration gate deferred to Phase 3)

## Phase 2: Implementation (Group A — SDK)
- [x] 1.1 Add `emit-arrow = ["dep:arrow"]` to exasol-udf-sdk; add `emit-arrow` to `connect-back`
- [x] 1.2 Add defaulted `emit_batch(&RecordBatch)` to `UdfContext` gated `#[cfg(feature = "emit-arrow")]`, default `Unimplemented`
- [x] 1.3 Unit test: default `emit_batch` returns `Unimplemented` (under `emit-arrow`)

## Phase 2: Implementation (Group C — runtime, depends on A)
- [x] 2.1 Add `emit-arrow` feature to exa-udf-runtime Cargo.toml; `connect-back` activates it
- [x] 2.2 Implement `EmitBuffer::push_batch` vectorised column-at-a-time encoder with row-granular 4MB split [expert]
- [x] 2.3 Add `output_meta` field to `HostContextBridge`; thread through `new`/`with_connection`; dispatch passes `&meta.output_columns` [expert]
- [x] 2.4 Implement `HostContextBridge::emit_batch` override gated `#[cfg(feature = "emit-arrow")]`
- [x] 2.5 Unit tests for push_batch (split, parity, nulls, byte estimate, type mismatch, tail bounded) [expert]

## Phase 2: Implementation (Group B — example crate, depends on A)
- [x] 3.1 Scaffold `test-udfs/emit-arrow-batch/` (Cargo.toml, features = ["emit-arrow"], arrow; cdylib)
- [x] 3.2 Implement `src/lib.rs`: drain input, build RecordBatch (Int64 + Utf8), emit_batch; add to workspace members

## Phase 2: Implementation (Group D — IT + docs, depends on B and C)
- [x] 4.1 Add `emit_arrow_batch_roundtrips` scenario in `crates/it/tests/db_roundtrip.rs`
- [x] 4.2 Wire the new `.so` into the IT harness upload list + ci-it-local.sh build list (mirror EMIT_BULK_LIB)
- [x] 5.1 Note `emit_batch` and the `emit-arrow` feature in docs/writing-a-udf

## Phase 3: Verification
- [x] V.1 Build, test, clippy, fmt (workspace + feature builds) — green (2 dispatch tests env-only: stale .so, pass on rebuild)
- [x] V.3 Code review — no correctness bugs; one spec-compliance fix routed (vectorise encode_slice per-column)
- [x] V.4 Fix: true per-column-once downcast in encode_slice/arrow_batch_to_value_rows + comment cleanup [expert]
- [x] V.5 ROOT CAUSE (confirmed by no-DB dlopen repro): `ctx.emit_batch(&RecordBatch)` passed a UDF-`.so`-built
      Arrow batch ACROSS the cdylib boundary → host downcast → SIGSEGV (two static `arrow` copies; B-002 hazard).
- [x] V.6 REDESIGN to Arrow IPC bytes: SDK `EmitBatch` blanket ext-trait serialises UDF-side →
      `UdfContext::emit_record_batch_ipc(&[u8])` ABI method → host `HostContextBridge` deserialises into its OWN
      RecordBatch → existing `push_batch` (unchanged). Only `&[u8]` crosses. Spec deltas + decision-log updated;
      raw-buffer optimisation filed as backlog B-005.
- [x] V.7 Boundary-safety gate (no-DB dlopen of the real `.so`, `tests/emit_arrow_dlopen.rs`): PASSES — the
      scenario that previously SIGSEGV'd now round-trips 3 rows cleanly via IPC.
- [x] V.2 Integration/E2E (scripts/ci-it-local.sh, live Exasol 2026.1.0): GREEN after the IPC redesign —
      all scenarios pass incl. `[it] scenario emit_arrow_batch ok`; `test result: ok. 1 passed`; `Done (rc=0)`.
