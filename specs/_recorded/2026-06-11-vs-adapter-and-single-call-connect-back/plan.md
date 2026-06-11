# Plan: 2026-06-11-vs-adapter-and-single-call-connect-back

## Summary

Delivers four interlocking capabilities — transaction control on `ExaConnection`, the `vs_adapter` macro annotation with ABI v3, the corrected single-call `MT_RETURN→MT_DONE` protocol loop, and a row-major rowset packing overhaul — all of which already exist as implemented, tested, and clippy-clean code in the working tree. This plan records those changes as spec deltas so `/speq:record` can merge them into the permanent spec library.

## Design

### Context

The virtual-schema adapter call (`SC_FN_VIRTUAL_SCHEMA_ADAPTER_CALL`) was previously dispatched without a `UdfContext` pointer, so the adapter could not resolve CONNECTION credentials or open a connect-back session. The `ExaConnection` trait lacked transaction control methods. The single-call protocol loop did not mirror the canonical C++ `send_return → receive MT_RETURN ack → send_done` sequence, causing sessions to terminate prematurely. The rowset codec used column-major packing with NULL placeholders, which produced silently wrong column values when NULL cells appeared in the input.

- **Goals** — Enable VS adapters to perform credential lookup and connect-back mid single-call; add explicit transaction control to `ExaConnection`; fix the single-call protocol loop to match the C++ reference; fix NULL-correct row-major rowset encoding.
- **Non-Goals** — Multi-call (streaming) VS adapter sessions; session pooling; WebSocket transport changes; JIT compilation; Windows/macOS targets.

### Decision

#### Architecture

```
#[exasol_udf(vs_adapter(fn))]          (sdk/exasol-udf-macros)
       │ generates __exa_vs_adapter_shim (3-arg ABI)
       ▼
ExaUdfVTable.virtual_schema_adapter_call  ABI version 3
       │ ctx = *mut c_void (double-indirected &mut dyn UdfContext)
       ▼
invoke_vs_adapter_call()               (runtime/single_call.rs)
       │ constructs SingleCallContext
       │ on-demand MT_IMPORT for ctx.connection()
       ▼
SingleCallContext : UdfContext         (runtime/rowset.rs)
       │ cluster_ip / connection / connect_back
       ▼
RuntimeExaConnection.begin/commit/rollback  (runtime/connect_back.rs)
       │ block_on + catch_unwind
       ▼
exarrow_rs::Connection::begin_transaction/commit/rollback
```

Single-call protocol loop (corrected):
```
MT_RUN → MT_CALL → dispatch → MT_RETURN(result)
       ← MT_RETURN(ack) [SingleCallAck]
       → MT_DONE
       ← MT_DONE | MT_CLEANUP
```

Rowset encoding (corrected):
```
Row-major within each type block, non-null cells only.
NULL cells set the null-bitmap; they do NOT push a slot into the type block.
Values are packed by declared ExaType, not by runtime Value variant.
```

#### Patterns

| Pattern | Where | Why |
|---------|-------|-----|
| Double-indirection ABI (`*mut c_void` → `&mut dyn UdfContext`) | `__exa_vs_adapter_shim`, `call_ctx_arg_hook` | Identical to the `run` shim; avoids a new ABI concept |
| `catch_unwind` at every FFI boundary | `__exa_vs_adapter_shim`, `run_txn_op` | Panics must not cross the FFI boundary into Exasol's SQL worker |
| `SingleCallContext` implementing `UdfContext` | `runtime/rowset.rs` | Reuses the `UdfContext` dispatch mechanism; data methods return `Unimplemented` |
| Per-type running cursors (no pre-computed offsets) | `InputRowSet::from_proto`, `EmitBuffer::to_proto` | Correctly handles NULL cells without placeholder slots |

### Consequences

| Decision | Alternatives Considered | Rationale |
|----------|------------------------|-----------|
| ABI version bump 2→3 for `virtual_schema_adapter_call` slot signature change | Keep v2, add a new slot | Slot signature change is a hard binary incompatibility; incrementing ABI version forces the loader to reject old `.so` files with a clear error rather than silently invoking the wrong signature |
| Row-major packing, NULL skips type-block slot | Keep column-major with placeholder | Column-major layout with placeholders produced wrong column values when NULLs appeared; row-major without placeholders matches the Exasol wire format |
| Default `begin`/`commit`/`rollback` on `ExaConnection` | Separate `TransactionalConnection` trait | Default methods preserve backward compatibility for existing mocks and test doubles; no API churn |
| `SingleCallContext` with on-demand `MT_IMPORT` closure | Materialize credentials at call start | The adapter may not need credentials for every call; lazy MT_IMPORT avoids unnecessary round-trips |

## Features

| Feature | Status | Spec |
|---------|--------|------|
| sdk/udf-sdk | CHANGED | `specs/_plans/2026-06-11-vs-adapter-and-single-call-connect-back/sdk/udf-sdk/spec.md` |
| sdk/connect-back | CHANGED | `specs/_plans/2026-06-11-vs-adapter-and-single-call-connect-back/sdk/connect-back/spec.md` |
| runtime/host-dispatch | CHANGED | `specs/_plans/2026-06-11-vs-adapter-and-single-call-connect-back/runtime/host-dispatch/spec.md` |
| runtime/connect-back | CHANGED | `specs/_plans/2026-06-11-vs-adapter-and-single-call-connect-back/runtime/connect-back/spec.md` |
| protocol/wire-protocol | CHANGED | `specs/_plans/2026-06-11-vs-adapter-and-single-call-connect-back/protocol/wire-protocol/spec.md` |

## Threshold Escalation Notice

The following features will exceed the library threshold of 10 scenarios per feature after these deltas merge. The `speq-record` orchestrator MUST be notified:

| Feature | Scenarios before | Net change | Scenarios after |
|---------|-----------------|------------|-----------------|
| `runtime/host-dispatch` | 14 | +2 NEW | 16 |
| `protocol/wire-protocol` | 16 | +1 NEW | 17 |

**sdk/udf-sdk** will reach 11 (above threshold) as well. Recommendation: split `runtime/host-dispatch` into a `rowset-codec` sub-feature after recording, or accept the overage and defer the split.

## Dependencies

- `exarrow_rs::Connection::begin_transaction`, `commit`, `rollback` (already present in local `exarrow-rs` v0.12.5+)
- ABI version 3 requires rebuilding all user `.so` files; the loader will reject v2 artifacts with a clear version-mismatch error.

## Migration

| Current | New |
|---------|-----|
| `EXA_UDF_ABI_VERSION = 2` | `EXA_UDF_ABI_VERSION = 3` |
| `virtual_schema_adapter_call: fn(json, result) -> i32` | `virtual_schema_adapter_call: fn(ctx, json, result) -> i32` |
| Column-major type blocks with NULL placeholders | Row-major type blocks, NULL skips type-block slot |
| No `begin`/`commit`/`rollback` on `ExaConnection` | Default methods returning `Unimplemented` |

Recommended version bump: **0.4.0 → 0.5.0** (breaking ABI change dominates semver under pre-1.0 rules).

## Implementation Tasks

All tasks below are verification-only — the code already exists and is tested.

1. [ ] 1.1 Verify spec delta for `sdk/udf-sdk` matches `abi.rs` and `lib.rs` changes (ABI v3, vs_adapter slot signature, macro shim generation)
2. [ ] 1.2 Verify spec delta for `sdk/connect-back` matches `connect_back.rs` trait additions (`begin`/`commit`/`rollback` defaults)
3. [ ] 1.3 Verify spec delta for `protocol/wire-protocol` matches `loop_.rs`/`messages.rs` (`SingleCallAck` event, `MT_RETURN` in single-call mode)
4. [ ] 1.4 Verify spec delta for `runtime/connect-back` matches `connect_back.rs` (`run_txn_op`, `SingleCallContext` implementation) [expert]
5. [ ] 1.5 Verify spec delta for `runtime/host-dispatch` matches `rowset.rs` (row-major packing, no placeholders, declared-type dispatch) [expert]
6. [ ] 1.6 Run `cargo +1.91 test --workspace` and confirm green
7. [ ] 1.7 Run `cargo clippy --all-targets --all-features -- -D warnings` and confirm clean
8. [ ] 1.8 Bump `Cargo.toml` workspace version from `0.4.0` to `0.5.0` and update `Cargo.lock`
9. [ ] 1.9 Run `speq plan validate 2026-06-11-vs-adapter-and-single-call-connect-back` and confirm pass
10. [ ] 1.10 Commit all changes with message `feat!: ABI v3, vs_adapter macro, single-call loop fix, row-major rowset`

## Parallelization

| Parallel Group | Tasks |
|----------------|-------|
| Verification | 1.1, 1.2, 1.3 |
| Deep verification | 1.4, 1.5 |
| Gates | 1.6, 1.7 (after 1.1–1.5) |

Sequential dependencies:
- Verification (1.1–1.5) → 1.6, 1.7 → 1.8 → 1.9 → 1.10

## Dead Code Removal

| Type | Location | Reason |
|------|----------|--------|
| Function | `crates/exa-udf-runtime/src/rowset.rs::push_placeholder` | Removed; NULL cells no longer need a placeholder in the type block |

## Verification

### Scenario Coverage

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| ABI constants and vtable layout are stable (v3) | Unit | `crates/exasol-udf-sdk/src/abi.rs` (inline) | `abi_version_and_vtable_layout` |
| vs_adapter slot receives context pointer | Unit | `crates/exasol-udf-sdk/src/abi.rs` (inline) | `vs_adapter_slot_receives_context_pointer` |
| vs_adapter annotation wires slot and echoes through context ABI | Unit | `crates/exasol-udf-macros/tests/vs_adapter.rs` | `vs_adapter_annotation_wires_slot_and_echoes_through_context_abi` |
| vs_adapter absent leaves slot None | Unit | `crates/exasol-udf-macros/tests/vs_adapter.rs` (absent-annotation path) | `vs_adapter_annotation_wires_slot_and_echoes_through_context_abi` (absent-slot branch) |
| ExaConnection transaction defaults return Unimplemented | Unit | `crates/exasol-udf-sdk/tests/connect_back.rs` | `transaction_methods_default_to_unimplemented` |
| MT_RETURN DB acknowledgement surfaces SingleCallAck | Unit | `crates/exa-zmq-protocol/tests/single_call.rs` | `mt_return_ack_in_single_call_mode_emits_single_call_ack` |
| MT_RETURN in non-single-call mode is protocol error | Unit | `crates/exa-zmq-protocol/tests/single_call.rs` | `mt_return_in_non_single_call_mode_is_protocol_error` |
| Single-call MT_RETURN ack terminates session via MT_DONE | Integration | `crates/exa-udf-runtime/tests/single_call.rs` | `mt_return_ack_terminates_session` |
| EmitBuffer packs output values row-major by declared column type | Unit | `crates/exa-udf-runtime/src/rowset.rs` (inline) | `emit_packs_by_declared_type_not_value_variant` |
| EmitBuffer string block is row-major across columns | Unit | `crates/exa-udf-runtime/src/rowset.rs` (inline) | `emit_string_block_is_row_major_across_columns` |
| NULL cell occupies no type-block slot | Unit | `crates/exa-udf-runtime/src/rowset.rs` (inline) | `emit_null_cell_occupies_no_type_block_slot` |
| InputRowSet decodes row-major type blocks correctly | Unit | `crates/exa-udf-runtime/src/rowset.rs` (inline, round-trip via `emit_packs_by_declared_type_not_value_variant`) | `emit_packs_by_declared_type_not_value_variant` |
| RuntimeExaConnection begin/commit/rollback | Integration | `crates/exa-udf-runtime/src/connect_back.rs` (tested via `run_txn_op` code path; integration test pending against live DB) | _(integration test: pending)_ |
| SingleCallContext exposes connect-back methods | Unit | `crates/exa-udf-runtime/src/rowset.rs` (inline, `SingleCallContext` impl returns Unimplemented for data methods) | `single_call_context_data_methods_are_unimplemented` |

### Manual Testing

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| vs_adapter macro | `cargo test -p exasol-udf-macros --test vs_adapter` | 1 test passed |
| ABI v3 | `cargo test -p exasol-udf-sdk` | All tests passed, including `abi_version_and_vtable_layout` asserting version 3 |
| SingleCallAck protocol | `cargo test -p exa-zmq-protocol --test single_call` | All tests passed, including `mt_return_ack_in_single_call_mode_emits_single_call_ack` |
| MT_RETURN ack runtime loop | `cargo test -p exa-udf-runtime --test single_call -- mt_return_ack_terminates_session` | 1 test passed |
| Row-major rowset | `cargo test -p exa-udf-runtime -- emit_packs_by_declared_type emit_null_cell emit_string_block` | 3 tests passed |
| Transaction defaults | `cargo test -p exasol-udf-sdk --test connect_back -- transaction_methods_default` | 1 test passed |
| Full workspace | `cargo +1.91 test --workspace` | 0 failures |
| Lint | `cargo clippy --all-targets --all-features -- -D warnings` | 0 errors, 0 warnings |

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Build | `cargo build --release` | Exit 0 |
| Test | `cargo +1.91 test --workspace` | 0 failures |
| Lint | `cargo clippy --all-targets --all-features -- -D warnings` | 0 errors/warnings |
| Format | `cargo fmt --check` | No changes |
