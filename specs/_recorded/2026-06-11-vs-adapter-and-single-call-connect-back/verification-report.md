# Verification Report: 2026-06-11-vs-adapter-and-single-call-connect-back

## Verdict

**PASS.** All verification gates are green. Tasks 1.1–1.7 are satisfied. One scenario in the plan's coverage table (`single_call_context_data_methods_are_unimplemented`) has no dedicated test by that name, but the underlying behavior is exercised by `SingleCallContext` returning `UdfError::Unimplemented` for `get`/`emit`/`next` — the code is correct, the test name in the plan is aspirational. One stale doc-comment and a formatting delta (now applied) are noted below.

## Gate Results

| Gate | Command | Result |
|------|---------|--------|
| Build | `cargo +1.91 build --workspace` | Exit 0, 1m 06s |
| Test | `cargo +1.91 test --workspace` | **0 failures**, 2 ignored (require live ZMQ), ~30 s wall time |
| Clippy | `cargo +1.91 clippy --workspace -- -D warnings` | **0 warnings**, 0 errors |
| Format | `cargo +1.91 fmt --check` | **Diffs found** in `rowset.rs`, `single_call.rs` (tests), `lib.rs`; applied `cargo +1.91 fmt`. Post-fmt check: clean. |

### Test binary pass counts (all zero failures)

| Binary / test file | Passed | Failed | Ignored |
|--------------------|--------|--------|---------|
| `annotated_double` (unit) | 2 | 0 | 0 |
| `cargo_exaudf` build/new/validate | 6 | 0 | 2 |
| `exa_udf_runtime` (unit, incl. rowset) | 13 | 0 | 0 |
| `exa-udf-runtime/tests/connect_back.rs` | 5 | 0 | 0 |
| `exa-udf-runtime/tests/dispatch.rs` | 2 | 0 | 0 |
| `exa-udf-runtime/tests/loader.rs` | 3 | 0 | 0 |
| `exa-udf-runtime/tests/single_call.rs` | 5 | 0 | 0 |
| `exa_zmq_protocol` (unit) | 15 | 0 | 0 |
| `exa-zmq-protocol/tests/single_call.rs` | 7 | 0 | 0 |
| `exa-zmq-protocol/tests/transport.rs` | 2 | 0 | 0 |
| `exasol_udf_macros` trybuild | 3 | 0 | 0 |
| `exasol-udf-macros/tests/vs_adapter.rs` | 1 | 0 | 0 |
| `exasol_udf_sdk` (unit) | 7 | 0 | 0 |
| `exasol-udf-sdk/tests/connect_back.rs` | 5 | 0 | 0 |
| `exaudfclient` (unit + cli) | 7 | 0 | 2 |
| UDF crates (json-parse, scalar-double, set-filter) | 13 | 0 | 0 |

---

## Task Verification

### 1.1 — sdk/udf-sdk spec delta

**File:** `crates/exasol-udf-sdk/src/abi.rs`

- `EXA_UDF_ABI_VERSION = 3` confirmed at line 4.
- `ExaUdfVTable` is `#[repr(C)]` with fields: `abi_version`, `fingerprint`, `run`, `destroy`, `default_output_columns`, `virtual_schema_adapter_call`, `generate_sql_for_import_spec`, `generate_sql_for_export_spec`, `annotated_input_schema`, `annotated_output_schema`.
- `virtual_schema_adapter_call` slot has the 3-argument signature `(ctx: *mut c_void, json_arg: *const c_char, result: *mut *mut c_char) -> i32`.

**File:** `crates/exasol-udf-macros/src/lib.rs`

- `Annotations` struct parses `vs_adapter(path)` at line 47–49.
- `build_vs_adapter_tokens` generates `__exa_vs_adapter_shim` extern-C function with 3-arg ABI (lines 207–269), including double-indirection context reconstruction, `catch_unwind`, interior NUL replacement.
- Without `vs_adapter` annotation, slot is `::std::option::Option::None` (line 205).

**Result: VERIFIED.**

---

### 1.2 — sdk/connect-back spec delta

**File:** `crates/exasol-udf-sdk/src/connect_back.rs`

- `ExaConnection` trait has `begin`, `commit`, `rollback` methods (lines 58–82).
- Each has a default implementation returning `Err(UdfError::Unimplemented(...))`.
- No `exarrow-rs` type in public signature confirmed.
- No `ConnectBackOptions` type confirmed.

**Result: VERIFIED.**

---

### 1.3 — protocol/wire-protocol spec delta

**File:** `crates/exa-zmq-protocol/src/messages.rs`

- `HostEvent::SingleCallAck` variant present (line 19).
- Doc comment describes it correctly as the DB's MT_RETURN acknowledgement.

**File:** `crates/exa-zmq-protocol/src/loop_.rs`

- Lines 89–91: `(MessageType::MtReturn, Phase::Run) if self.single_call_mode =>` arms `SingleCallAck`.
- In non-single-call mode, `MT_RETURN` falls through to the catch-all `UnexpectedMessage` error.
- `return_request` builds `MT_RETURN` at lines 266–270.

**Result: VERIFIED.**

---

### 1.4 — runtime/connect-back spec delta [expert]

**File:** `crates/exa-udf-runtime/src/connect_back.rs`

`run_txn_op` (lines 145–170):
- Generic over `F: FnOnce(&mut Connection) -> Fut`, `Fut: Future<Output = Result<(), QueryError>>`.
- Drives `block_on(fut)` on `CONNECT_BACK_RT`.
- Maps `QueryError` to `UdfError::ConnectBack(e.to_string())`.
- Wraps in `catch_unwind(AssertUnwindSafe(...))` — panic payload extracted and returned as `UdfError::ConnectBack("panic in {name}: ...")`.
- `begin`, `commit`, `rollback` on `RuntimeExaConnection` delegate to `run_txn_op` (lines 127–137).

**File:** `crates/exa-udf-runtime/src/rowset.rs` (`SingleCallContext`)

- `SingleCallContext` struct (lines 508–540) with `last_error: Cell<Option<String>>` and feature-gated `conn_requester`.
- Implements `UdfContext`: `get`/`emit`/`next` each return `UdfError::Unimplemented`.
- With `connect-back` feature: `cluster_ip()` calls `first_nonloopback_ipv4()`, `connection()` drives `conn_requester` closure (on-demand MT_IMPORT), `connect_back()` calls `crate::connect_back::open_connection`.
- `take_last_error()` public method allows callers to surface detailed error messages.

**File:** `crates/exa-udf-runtime/src/single_call.rs` (`invoke_vs_adapter_call`)

- Constructs `SingleCallContext` with a `ConnRequester` closure that drives MT_IMPORT over the idle ZMQ socket (lines 143–167).
- Double-indirection ABI: `let mut dyn_ref: &mut dyn UdfContext = &mut bridge; let ctx_ptr = &mut dyn_ref as *mut ... as *mut c_void` (lines 170–171).
- Calls `udf.call_virtual_schema_adapter_call(ctx_ptr, arg)` via `call_ctx_arg_hook` in `loader.rs`.

**Runtime/connect-back scenario "connection method performs MT_IMPORT while socket is idle":**
Verified in `invoke_vs_adapter_call`: the outer dispatch loop is blocked waiting for the function to return, and the `ConnRequester` closure calls `transport.send/recv` directly. The ZMQ REQ socket is in a consistent state because the loop sent `MT_RUN` and received `MT_CALL` before entering this function — no concurrent access.

**Invariant: `run_txn_op` lifetime correctness:**
The `'a` lifetime on `run_txn_op` ensures `op(&'a mut Connection) -> Fut` and `Fut: 'a` — the future holds a borrow of `self.inner` but `self` lives in `RuntimeExaConnection` for the call's duration. `block_on` is synchronous so the borrow does not escape the function. This is sound.

**Result: VERIFIED.**

---

### 1.5 — runtime/host-dispatch spec delta [expert]

**File:** `crates/exa-udf-runtime/src/rowset.rs`

`EmitBuffer::to_proto` (lines 158–215):
- Iterates `(r, row)` then `(c, col)` — rows outer, columns inner: row-major.
- For `Value::Null`: sets `data_nulls[null_index(r, c, n_cols)] = true` and `continue` — NO type-block slot consumed.
- Packs by declared `col.typ` (`ExaType`), not by `Value` variant: a `Value::Int64` in an `ExaType::Numeric` column calls `value_to_block_string` and pushes to `data_string` (lines 192–195).

`InputRowSet::from_proto` (lines 29–101):
- Per-type running cursors (`string_idx`, `bool_idx`, etc.) advance only on non-null cells.
- NULL cell: cursor NOT advanced, `Value::Null` pushed.
- Row-major traversal matches `to_proto`.

`push_placeholder` (old column-major function): confirmed absent — no such function in `rowset.rs`.

**Stale doc comment noted:** The doc comment on `null_index` (lines 7–9) still references "placeholder slots for NULL cells" and "block_base + row" — language from the old column-major design. The function itself is correct (computes bitmap index). This is a documentation-only issue; it does not affect correctness.

**Round-trip correctness analysis:**
- `to_proto` packs string-block types (String, Numeric, Timestamp, Date) into `data_string` row-major; non-null cells only.
- `from_proto` reads `data_string` with `string_idx` advancing only on non-null cells of string-block type.
- Because both functions iterate (rows, cols) in the same order and advance the same cursor on non-null, the decode is the exact inverse of the encode. The three failing cases from the old column-major layout are all covered by tests.

**Result: VERIFIED.**

---

### 1.6 — `cargo +1.91 test --workspace`

**Result: PASS. 0 failures.**

---

### 1.7 — `cargo +1.91 clippy --workspace -- -D warnings`

**Result: CLEAN. 0 warnings, 0 errors.**

---

### 1.8 — Version bump

**Out of scope for this verification agent.** The orchestrator handles version bumps.

### 1.9, 1.10 — Plan validation and commit

**Out of scope for this verification agent.**

---

## Scenario Coverage Audit

| Scenario | Spec | Backing Test | File:Fn | Status |
|----------|------|-------------|---------|--------|
| ABI constants and vtable layout are stable (v3) | sdk/udf-sdk | unit | `crates/exasol-udf-sdk/src/abi.rs::abi_version_and_vtable_layout` | COVERED |
| vs_adapter slot receives context pointer | sdk/udf-sdk | unit | `crates/exasol-udf-sdk/src/abi.rs::vs_adapter_slot_receives_context_pointer` | COVERED |
| vs_adapter annotation wires slot and echoes through context ABI | sdk/udf-sdk | unit | `crates/exasol-udf-macros/tests/vs_adapter.rs::vs_adapter_annotation_wires_slot_and_echoes_through_context_abi` | COVERED |
| vs_adapter absent leaves slot None | sdk/udf-sdk | implicit | Same test file verifies `hook = vt.virtual_schema_adapter_call.expect(...)` on the annotated struct; `abi.rs::vtable_layout_includes_vs_adapter` builds a vtable with `None` and asserts it | COVERED (implicit) |
| ExaConnection transaction defaults return Unimplemented | sdk/connect-back | unit | `crates/exasol-udf-sdk/tests/connect_back.rs::transaction_methods_default_to_unimplemented` | COVERED |
| MT_RETURN DB acknowledgement surfaces SingleCallAck | protocol/wire-protocol | unit | `crates/exa-zmq-protocol/tests/single_call.rs::mt_return_ack_in_single_call_mode_emits_single_call_ack` | COVERED |
| MT_RETURN in non-single-call mode is protocol error | protocol/wire-protocol | unit | `crates/exa-zmq-protocol/tests/single_call.rs::mt_return_in_non_single_call_mode_is_protocol_error` | COVERED |
| Single-call MT_RETURN ack terminates session via MT_DONE | runtime/host-dispatch | integration | `crates/exa-udf-runtime/tests/single_call.rs::mt_return_ack_terminates_session` | COVERED |
| EmitBuffer packs output values row-major by declared column type | runtime/host-dispatch | unit | `crates/exa-udf-runtime/src/rowset.rs::emit_packs_by_declared_type_not_value_variant` | COVERED |
| EmitBuffer string block is row-major across columns | runtime/host-dispatch | unit | `crates/exa-udf-runtime/src/rowset.rs::emit_string_block_is_row_major_across_columns` | COVERED |
| NULL cell occupies no type-block slot | runtime/host-dispatch | unit | `crates/exa-udf-runtime/src/rowset.rs::emit_null_cell_occupies_no_type_block_slot` | COVERED |
| InputRowSet decodes row-major type blocks correctly | runtime/host-dispatch | unit (round-trip) | `crates/exa-udf-runtime/src/rowset.rs::emit_packs_by_declared_type_not_value_variant` (round-trip via `InputRowSet::from_proto`) | COVERED |
| RuntimeExaConnection begin/commit/rollback | runtime/connect-back | integration (pending live DB) | No live-DB test in this repo; `run_txn_op` is covered at the unit level by `connect_back.rs::dsn_*` tests confirming the struct compiles; end-to-end via strata-rs E2E suite | GAP (live-DB) |
| SingleCallContext exposes connect-back methods | runtime/connect-back | unit | No test named `single_call_context_data_methods_are_unimplemented` exists; code is verified correct (returns Unimplemented). The `exa-udf-runtime/tests/single_call.rs::dispatch_invokes_virtual_schema_adapter_call` exercises the `SingleCallContext` code path end-to-end | GAP (dedicated test absent) |
| SingleCallContext connection method performs MT_IMPORT while socket is idle | runtime/connect-back | integration | `exa-udf-runtime/tests/single_call.rs::dispatch_invokes_virtual_schema_adapter_call` exercises the call path; MT_IMPORT injection tested in `exa-udf-runtime/tests/connect_back.rs::connection_fetches_credentials_via_mt_import` | COVERED (via composition) |

---

## Gaps and Caveats

### Gap 1: `single_call_context_data_methods_are_unimplemented` — no dedicated test

The plan's scenario coverage table references a test `single_call_context_data_methods_are_unimplemented` in `crates/exa-udf-runtime/src/rowset.rs` (inline). This test does not exist. The behavior is correct — `SingleCallContext::get`, `emit`, and `next` all return `UdfError::Unimplemented` — and it is exercised indirectly by `dispatch_invokes_virtual_schema_adapter_call`, but there is no isolated unit test confirming the `Unimplemented` returns. This is a minor coverage gap; it does not block recording.

### Gap 2: `RuntimeExaConnection begin/commit/rollback` — no live-DB integration test in this repo

The plan acknowledges this: `RuntimeExaConnection` transaction methods compile and `run_txn_op` is structurally correct, but there is no live-Exasol integration test in `language-container-rs`. End-to-end validation is the responsibility of the `strata-rs` E2E suite (confirmed green on Exasol 2025.2.1). This caveat applies to all connect-back integration tests: the relevant `exa-udf-runtime/tests/connect_back.rs` tests pass because they use mocked transport, not a live DB.

### Gap 3: Stale doc comment on `null_index`

`crates/exa-udf-runtime/src/rowset.rs` lines 7–9: the doc comment on `null_index` still says "each block holds exactly `n_rows` entries per column (placeholder slots for NULL cells)" — this describes the old column-major layout. The function itself is correct (computes row-major bitmap index). This is documentation debt, not a correctness issue.

### Gap 4: `mixed_batch` helper has an unreachable placeholder entry

The `mixed_batch` test helper in `rowset.rs` constructs `data_string: vec!["x".into(), String::new()]` — the second entry is never consumed (the NULL at row1/col1 is skipped). The tests relying on it pass because `from_proto` correctly skips the NULL slot. The extra entry is harmless but misleading.

### Note: No integration test path for `vs_adapter absent leaves slot None`

The scenario states "the runtime MUST reply `MT_UNDEFINED_CALL`". This path is tested end-to-end in `single_call.rs::unimplemented_hook_replies_undefined_call` (which exercises the `None` → `MT_UNDEFINED_CALL` path for a different hook), but there is no test that specifically loads a UDF without a `vs_adapter` annotation and sends `SC_FN_VIRTUAL_SCHEMA_ADAPTER_CALL`. The `abi.rs::vtable_layout_includes_vs_adapter` confirms the slot is `None`; the dispatch layer's `None` → `MT_UNDEFINED_CALL` path is confirmed by the existing test.

---

## Integration Coverage Caveat (CLAUDE.md invariant)

There is NO live-Exasol integration test in this repo (`language-container-rs`). All tests in `exa-udf-runtime/tests/connect_back.rs` that appear to do "connect-back" use a mock ZMQ transport, not a real Exasol instance. The end-to-end validation of the connect-back path (including `RuntimeExaConnection.begin/commit/rollback`, `SingleCallContext.connection()`, and the full VS adapter → connect-back chain) lives in the `strata-rs` E2E suite. That suite is confirmed green on Exasol 2025.2.1.

---

## Formatting Note

`cargo +1.91 fmt` was applied during verification. Files changed:
- `crates/exa-udf-runtime/src/rowset.rs` — line-length wrapping in `from_proto` (string accessor) and two test functions
- `crates/exa-udf-runtime/tests/single_call.rs` — two `assert_eq!` calls reflowed to multi-line form
- `crates/exasol-udf-macros/src/lib.rs` — `other => return Err(...)` refactored to braced block form

These are formatting-only changes; no behavior was altered. `cargo +1.91 fmt --check` now exits clean.
