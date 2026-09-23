# Plan: add-udf-cleanup-hook

## Summary

Adds an optional Rust UDF cleanup hook, `#[exasol_udf(cleanup(f))]`, that runs once per UDF process at the protocol points where the reference client runs Python and Java `cleanup` (issue #119). The hook's error reaches the database, and after a `run` error the client reports both errors.

## Design

### Context

The `ExaUdfVTable.destroy` slot takes no context, reports no error, and the macro always fills it with an empty function. The runtime calls it only after `MT_FINISHED` or the error `MT_CLOSE`, so no error from it can reach the database. The reference client runs the user's `cleanup` inside `vm->shutdown()`: before `MT_FINISHED` on the normal path, and before the error `MT_CLOSE` for every failure after VM construction (`exaudflib_main.cc:63-72, 240, 262, 270-286`).

- **Goals**: an author annotates a cleanup function, the runtime calls it once per process whenever dispatch started, and a cleanup error fails the statement with the author's text.
- **Non-Goals**: an outcome flag (the user declined it), a `UdfRun` trait method, CONNECTION lookups over `MT_IMPORT` during cleanup, and any change to the pure protocol state machine.

### Decision

The `destroy` slot becomes an optional `cleanup` slot with the `run` slot's `(ctx, error_out) -> i32` signature, and the ABI version goes `10 → 11`. The dispatchers stop sending the session's final message. `Runtime::run` runs the hook through a new `cleanup` module after either dispatcher returns, then sends `MT_FINISHED` or one error `MT_CLOSE`.

#### Architecture

```
Runtime::run (lib.rs)
  handshake → load → validate output shape / annotated schema ──fail──▶ MT_CLOSE (hook skipped)
        │
        ▼
  dispatch::run_udf  |  single_call::run_single_call
        │  Ok  on MT_CLEANUP
        │  Err on a run error, hook error, DB MT_CLOSE, or protocol error
        │  (sends neither MT_FINISHED nor the error MT_CLOSE)
        ▼
  cleanup::run_hook(udf, meta, outcome)                      new module
        │  CleanupContext::new(handshake, input_iter, output_iter)   new type, rowset.rs
        │  LoadedUdf::cleanup(ctx) ──▶ vtable.cleanup: Option<fn(ctx, error_out) -> i32>
        │  fold: original error first, cleanup error second
        ▼
  Ok  ──▶ MT_FINISHED, await the echo
  Err ──▶ MT_CLOSE "F-UDF-CL-RUST-9001: ..."
```

The `cleanup` module has one responsibility: it runs the optional hook once at session end and folds the hook's failure into the session outcome. It hides the context construction, the double-indirected context pointer, and the error fold. `lib.rs` keeps the choice of the final wire message. `loader.rs` keeps the conversion of a slot's return code and out-pointer into a `RuntimeError`. No module outside `cleanup.rs` changes when the fold format or the context construction changes.

`CleanupContext` serves the `UdfContext` surface that stays legal after `MT_CLEANUP`. It lives in `rowset.rs` beside `SingleCallContext`. It adds no interface: the hook sees the `UdfContext` trait through the same double-indirected pointer as every other hook. It owns one decision: the cleanup phase sends nothing on the control channel, so `ctx.connection(name)` refuses in its own method body. Neither `cleanup.rs` nor `SingleCallContext` encodes that rule. It shares the handshake accessors and the `cluster_ip` and `connect_back` bodies with the other two contexts through the `rowset.rs` delegation macros.

#### Patterns

| Pattern | Where | Why |
|---------|-------|-----|
| One shim builder for the `(ctx, error_out) -> i32` shape | `crates/exasol-udf-macros/src/lib.rs` | the `run` and `cleanup` shims map `Ok`, `Err`, and panic to `0`, `1`, and `2` by construction |
| One slot-call helper | `crates/exa-udf-runtime/src/loader.rs` | `run` and `cleanup` share the out-pointer ownership and the `UDF <slot> returned error code <rc>: <text>` message |
| Single teardown owner | `lib.rs` plus `cleanup.rs` | every dispatch exit passes one step, so no exit path can skip the hook |
| One `UdfContext` implementation per protocol phase | `CleanupContext` beside `SingleCallContext` in `rowset.rs` | the phase after `MT_CLEANUP` forbids `MT_IMPORT`, so the type that serves it refuses `connection` by construction, and `SingleCallContext` stays unchanged |
| Shared delegation macros | `delegate_handshake_meta!` and the split connect-back macros in `rowset.rs` | the three contexts share one body per handshake accessor and per `cluster_ip`/`connect_back`, and only `connection` differs |

### Consequences

| Decision | Alternatives Considered | Rationale |
|----------|------------------------|-----------|
| Repurpose `destroy` as `cleanup: Option<...>` with the `run` shape, and bump the ABI to 11 | a new slot beside `destroy`, or keeping the field name `destroy` | one slot and one name per lifecycle concept, and the field type change forces the bump under every option |
| `Runtime::run` owns the teardown after the dispatcher returns | a hook call at each exit site inside both dispatchers | the exits include `?`-propagated protocol errors and a DB `MT_CLOSE`, so a per-site call repeats one decision at every site |
| Skip the hook when the output-shape or schema check fails | run the hook on every loaded `.so` | the reference client skips cleanup when VM construction fails (`exaudflib_main.cc:195-202`) |
| Cleanup gets a dedicated `CleanupContext` whose `connection(name)` refuses without sending `MT_IMPORT` | `SingleCallContext` with a live or a refusing credential requester, a live requester after a `run` error only, or a session cache of resolved connections | after `MT_CLEANUP` the engine accepts only `MT_FINISHED` or `MT_CLOSE`, a separate type gives that phase rule one owner, and `ctx.connect_back(conn)` needs no control-channel message |
| One `MT_CLOSE` carries the original error first and the cleanup error second | the cleanup error replaces the original | the reference `F-UDF-CL-LIB-1111` message keeps this order, and the original error is the root cause |

## Features

| Feature | Status | Spec |
|---------|--------|------|
| sdk/udf-abi | CHANGED | `specs/_plans/add-udf-cleanup-hook/sdk/udf-abi/spec.md` |
| sdk/udf-macro | CHANGED | `specs/_plans/add-udf-cleanup-hook/sdk/udf-macro/spec.md` |
| runtime/dispatch-run-loop | CHANGED | `specs/_plans/add-udf-cleanup-hook/runtime/dispatch-run-loop/spec.md` |
| runtime/dispatch-single-call | CHANGED | `specs/_plans/add-udf-cleanup-hook/runtime/dispatch-single-call/spec.md` |

## Impact

Rust UDF authors gain `#[exasol_udf(cleanup(f))]` with `f: fn(&mut dyn UdfContext) -> Result<(), UdfError>`. A UDF without a `cleanup` section behaves as before on the wire.

Behavior for a UDF that registers a cleanup hook:

- The hook runs once per UDF process, after that process's last group or call, and after a `run` error, a single-call error, or a DB `MT_CLOSE`.
- A cleanup error fails the statement with an `F-UDF-CL-RUST-9001` message carrying the hook's text. After an earlier error, the message carries both texts.
- Inside the hook, `ctx.connection(name)` always returns an error. The error tells the author to resolve the `ConnectionObject` during `run` and keep it, for example in a `static`. `ctx.connect_back(conn)` works with such an object.

Breaking changes for downstream users:

- `EXA_UDF_ABI_VERSION` goes `10 → 11`. Every deployed `.so` must be rebuilt, and a stale one fails the loader check with `AbiMismatch`.
- A hand-written `ExaUdfVTable` literal that bypasses the macro must rename `destroy` to `cleanup` and set `None` or a `(ctx, error_out) -> i32` function.
- The release needs a minor `[workspace.package].version` bump, the matching `exasol-udf-sdk` pin in `[workspace.dependencies]`, and a regenerated `Cargo.lock` in the same PR.

## Dependencies

No new third-party crates enter the workspace.

## Migration

| Current | New |
|---------|-----|
| `destroy: unsafe extern "C" fn()`, always set | `cleanup: Option<unsafe extern "C" fn(*mut c_void, *mut *mut c_char) -> i32>`, `None` unless annotated |
| `.so` built against `exasol-udf-sdk` 0.29.x (ABI 10) | rebuilt against 0.30.0 (ABI 11) |

## Implementation Tasks

1. **Cleanup slot and macro**
   1. 1.1 In `crates/exasol-udf-sdk/src/abi.rs`, replace `destroy` with `cleanup: Option<unsafe extern "C" fn(ctx, error_out) -> i32>` at the same field position, document the `run`-slot contract on it, and bump `EXA_UDF_ABI_VERSION` to 11. Update the vtable literals in `abi_tests.rs` and add `cleanup_slot_takes_context_and_abi_version_is_eleven`
   2. 1.2 Parse a `cleanup(path)` section into `Annotations`, and add `cleanup` to the unknown-section error message and to `crates/exasol-udf-macros/tests/trybuild/unknown_annotation_section.stderr`
   3. 1.3 Extract the `run` shim generation into one builder for the `(ctx, error_out) -> i32` shape. Generate `__exa_cleanup_shim_<NAME>` through it when `cleanup` is present, wire `Some(shim)` or `None` into the `cleanup` slot, and delete the unconditional destroy shim [expert]
   4. 1.4 Add `crates/exasol-udf-macros/tests/cleanup.rs` with the five macro tests named in Scenario Coverage
   5. 1.5 Rename `destroy` to `cleanup` in the hand-written vtables: `test-udfs/single-call-fixture/src/lib.rs` (set `None`, delete `destroy_shim`), the `VTableProbe` offset-24 field in `crates/cargo-exasol-udf/src/validate.rs`, and the vtable source templates in `crates/cargo-exasol-udf/tests/validate.rs`

2. **Session teardown**
   1. 2.1 In `crates/exa-udf-runtime/src/loader.rs`, replace `LoadedUdf::destroy` with `LoadedUdf::cleanup(ctx) -> Option<Result<(), RuntimeError>>`. Move the return-code and out-pointer handling from `dispatch::invoke_run` into one loader helper that `run` and `cleanup` share, keeping the exact `UDF run returned error code <rc>[: <text>]` message and allocating nothing on the per-row success path. Update the vtable templates in `loader_tests.rs` and `tests/loader.rs`
   2. 2.2 In `crates/exa-udf-runtime/src/rowset.rs`, add `CleanupContext` beside `SingleCallContext` and implement `UdfContext` for it. Give it the handshake metadata and the iteration axes, with no credential requester and no lifetime parameter. Split `delegate_connect_back_hooks!` into a `connection` macro and a `cluster_ip` plus `connect_back` macro. Keep both invoked in `HostContextBridge` and `SingleCallContext`. Invoke `delegate_handshake_meta!()` and the `cluster_ip` plus `connect_back` macro in `CleanupContext`. Write its `connection` body without a `connect-back` gate. It returns `UdfError::ConnectBack` immediately, records the error, and sends nothing. Its message states that CONNECTION lookups are unavailable during cleanup. The message also tells the author to resolve the `ConnectionObject` during `run()` and keep it, for example in a `static`. Return `Unimplemented` from `next`, `get`, and `emit`, and derive `input_type`/`output_type` from the iteration axes, as `SingleCallContext` does. State the phase rule and the resolve-during-`run` pattern in the type's doc comment. Add `cleanup_context_refuses_connection_lookup` to `rowset_tests.rs` without a feature gate. Then add `crates/exa-udf-runtime/src/cleanup.rs`. It builds a `CleanupContext` from `HandshakeMeta::from(meta)`, `meta.input_iter()`, and `meta.output_iter()`, invokes the hook, and appends a context-recorded error. It folds the result into the dispatch outcome with the original error first. Add `cleanup_tests.rs` with `fold_keeps_the_original_error_first` [expert]
   3. 2.3 Remove the trailing `finished_reply` exchange from `run_udf` and `run_single_call`. In `Runtime::run`, run the cleanup step after either dispatcher returns, then send `MT_FINISHED` on success or the error close on failure. Delete the `destroy` calls at the two pre-dispatch validation sites, and update the teardown doc comments in `lib.rs`, `dispatch.rs`, `single_call.rs`, and `loader.rs` [expert]
   4. 2.4 Add the `test-udfs/cleanup-hook` fixture crate with the entry points in the table below. Add it to the root `members` and `default-members`, as an `exa-udf-runtime` `[dev-dependencies]` entry, and to the CI "Build UDF .so artifacts (release)" `-p` allowlist
   5. 2.5 Add the run-loop mock tests to `crates/exa-udf-runtime/tests/dispatch.rs`. Leave `cleanup_connection_lookup_is_refused_without_mt_import` without a feature gate, because `CleanupContext::connection` refuses in every build. Update `mid_group_cleanup_ends_session_cleanly` to receive and answer the client's `MT_FINISHED`
   6. 2.6 Add `single_call_cleanup_runs_before_finished` and `single_call_error_still_runs_cleanup` to `crates/exa-udf-runtime/tests/single_call.rs`, both without a feature gate
   7. 2.7 Upload `libcleanup_hook.so` in `crates/it/tests/db_roundtrip.rs` and add the five live scenarios named in Scenario Coverage, each printing `[it] scenario <name> ok`
   8. 2.8 Document the `cleanup(path)` section, the hook's signature, its once-per-process timing, its error reporting, and the `connection` versus `connect_back` rule with the resolve-during-`run` pattern in `docs/writing-a-udf.md` §2. Update the cleanup step in `docs/protocol.md` §3 and the lifecycle line in `specs/architecture.md`

3. **Release hygiene**
   1. 3.1 Bump `[workspace.package].version` to 0.30.0, update the pinned `exasol-udf-sdk` entry in `[workspace.dependencies]` to match, and commit the regenerated `Cargo.lock`. The bump changes `EXA_SDK_FINGERPRINT`, so every `test-udfs/*.so` MUST be rebuilt before the Integration checklist step runs

The fixture for tasks 2.4 to 2.7 exports one entry point per behavior. The `.so` stays mapped across the tests of one test process, so each entry point that keeps counters in statics serves exactly one runtime test.

| Entry point | Registration | `run` | Cleanup hook | Tests |
|-------------|--------------|-------|--------------|-------|
| `CLEANUP_OK` | SCALAR RETURNS | returns `x` | `Ok(())` | `successful_cleanup_precedes_mt_finished`, `cleanup_ok_statement_succeeds` |
| `CLEANUP_REPORTS` | SET EMITS | counts the group and its rows in statics, emits the row count | after at least one group: `Err("cleanup ran: script=<script_name> groups=<g> rows=<r> io_rejected=<bool>")`, where `io_rejected` is true when `ctx.next()`, `ctx.get(0)`, and `ctx.emit(..)` all fail. With no group: `Ok(())` | `cleanup_runs_once_after_the_last_group`, `cleanup_reports_per_process_counts` |
| `CLEANUP_AFTER_RUN_ERROR` | SCALAR EMITS, `input(x: i64)` | `Err("run failed on purpose")` | `Err("cleanup failed on purpose")` | `run_error_runs_cleanup_and_reports_both_errors`, `db_close_runs_cleanup_before_relaying_the_close`, `cleanup_runs_when_no_group_ran`, `output_shape_mismatch_skips_cleanup`, `schema_mismatch_skips_cleanup`, `run_and_cleanup_errors_both_surface` |
| `CLEANUP_CONNECTION` | SCALAR EMITS | no output | returns the error of `ctx.connection("CB_SELF")` unchanged, which is the `CleanupContext` refusal | `cleanup_connection_lookup_is_refused_without_mt_import` |
| `CLEANUP_CONNECT_BACK` | SCALAR RETURNS | resolves `ctx.connection("CB_SELF")` once into a static, returns `x` | after a resolved connection: sets `connection_refused` to true only when `ctx.connection("CB_SELF")` fails with text naming the cleanup phase. Reads `SELECT CAST(42 AS BIGINT)` over `ctx.connect_back(&conn)` with the static object, and returns `Err("cleanup connect-back read <n> connection_refused=<bool>")`. Without one: `Ok(())` | `cleanup_connects_back_with_a_resolved_connection_object` |
| `EXPORT_CLEANUP` | single-call, `export_spec(...)` returning `SELECT 1` | no output | returns `Err` with the text `cleanup ran after export_spec: ` followed by the error text of `ctx.connection("CB_SELF")` | `single_call_cleanup_runs_before_finished`, `single_call_error_still_runs_cleanup`, `export_into_script_fails_on_cleanup_error` |

## Parallelization

| Group | Tasks | Depends on | Knowledge |
|-------|-------|------------|-----------|
| A: Cleanup slot and macro | 1.1-1.5 | — | spec deltas `sdk/udf-abi`, `sdk/udf-macro`; `crates/exasol-udf-sdk/src/{abi.rs,abi_tests.rs}`, `crates/exasol-udf-macros/src/lib.rs`, `crates/exasol-udf-macros/tests/{cleanup.rs,trybuild/unknown_annotation_section.stderr}`, `test-udfs/single-call-fixture/src/lib.rs`, `crates/cargo-exasol-udf/src/validate.rs`, `crates/cargo-exasol-udf/tests/validate.rs` |
| B: Session teardown | 2.1-2.8 | A (the `cleanup` slot type, and the `cleanup(path)` section the fixture uses) | spec deltas `runtime/dispatch-run-loop`, `runtime/dispatch-single-call`; `crates/exa-udf-runtime/src/{loader.rs,loader_tests.rs,rowset.rs,rowset_tests.rs,cleanup.rs,cleanup_tests.rs,dispatch.rs,single_call.rs,lib.rs}`, `crates/exa-udf-runtime/tests/{dispatch.rs,single_call.rs,loader.rs}`, `crates/exa-udf-runtime/Cargo.toml`, `test-udfs/cleanup-hook/`, root `Cargo.toml` members, `.github/workflows/ci.yml`, `crates/it/tests/db_roundtrip.rs`, `docs/writing-a-udf.md`, `docs/protocol.md`, `specs/architecture.md` |
| C: Release hygiene | 3.1 | A, B | root `Cargo.toml` version and pin, `Cargo.lock` |

Group A leaves `exa-udf-runtime` uncompiled until group B replaces `LoadedUdf::destroy`. Group A verifies with `cargo test -p exasol-udf-sdk -p exasol-udf-macros -p cargo-exasol-udf`, and group B restores the full workspace build.

## Dead Code Removal

| Type | Location | Reason |
|------|----------|--------|
| Function | `crates/exa-udf-runtime/src/loader.rs::LoadedUdf::destroy` | replaced by `LoadedUdf::cleanup` |
| Generated item | the unconditional `__exa_destroy_shim_<NAME>` in `crates/exasol-udf-macros/src/lib.rs` | the cleanup shim exists only when annotated |
| Function | `test-udfs/single-call-fixture/src/lib.rs::destroy_shim` | the fixture sets `cleanup: None` |
| Call | the trailing `finished_reply` exchange in `dispatch::run_udf` and `single_call::run_single_call` | `Runtime::run` sends `MT_FINISHED` after the cleanup step |
| Code | the return-code and out-pointer formatting in `dispatch::invoke_run` | moved into the shared loader helper |

## Verification

### Scenario Coverage

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| sdk/udf-abi: cleanup annotation wires the cleanup slot | Integration | `crates/exasol-udf-macros/tests/cleanup.rs` | `cleanup_annotation_wires_slot_and_maps_outcomes` |
| sdk/udf-abi: cleanup absent leaves the slot None | Integration | `crates/exasol-udf-macros/tests/cleanup.rs` | `omitted_cleanup_leaves_slot_none` |
| sdk/udf-abi: cleanup absent leaves the slot None | Integration | `crates/exa-udf-runtime/tests/dispatch.rs` | `scalar_dispatch_full_protocol` (unchanged) |
| sdk/udf-abi: The cleanup slot replaces destroy in place, bumping the ABI version | Unit | `crates/exasol-udf-sdk/src/abi_tests.rs` | `cleanup_slot_takes_context_and_abi_version_is_eleven` |
| sdk/udf-macro: exasol_udf macro generates the entry point and vtable | Integration | `crates/exasol-udf-macros/tests/cleanup.rs` | `cleanup_shim_carries_the_entry_suffix` |
| sdk/udf-macro: function name is translated to UPPER_SNAKE_CASE SQL name | Integration | `crates/exasol-udf-macros/tests/cleanup.rs` | `cleanup_shim_carries_the_entry_suffix` |
| sdk/udf-macro: name attribute overrides the SQL entry point name | Integration | `crates/exasol-udf-macros/tests/cleanup.rs` | `name_combines_with_cleanup_section` |
| sdk/udf-macro: Two exasol_udf annotations with distinct names produce independent entry points | Integration | `crates/exasol-udf-macros/tests/cleanup.rs` | `distinct_entries_get_independent_cleanup_shims` |
| sdk/udf-macro: Macro rejects an unknown annotation section by name | Integration | `crates/exasol-udf-macros/tests/trybuild/` | `unknown_annotation_section.rs` |
| runtime/dispatch-run-loop: UDF error closes the session with a prefixed message | Integration | `crates/exa-udf-runtime/tests/dispatch.rs` | `udf_error_closes_session_with_prefixed_message` (unchanged), `run_error_runs_cleanup_and_reports_both_errors` |
| runtime/dispatch-run-loop: The cleanup hook runs once after the last group, before MT_FINISHED | Integration | `crates/exa-udf-runtime/tests/dispatch.rs` | `cleanup_runs_once_after_the_last_group`, `successful_cleanup_precedes_mt_finished`, `cleanup_runs_when_no_group_ran` |
| runtime/dispatch-run-loop: The cleanup hook runs once after the last group, before MT_FINISHED | Integration | `crates/it/tests/db_roundtrip.rs` | `cleanup_ok_statement_succeeds`, `cleanup_reports_per_process_counts` |
| runtime/dispatch-run-loop: A cleanup hook error fails the statement instead of MT_FINISHED | Integration | `crates/exa-udf-runtime/tests/dispatch.rs` | `cleanup_runs_once_after_the_last_group` |
| runtime/dispatch-run-loop: A cleanup hook error fails the statement instead of MT_FINISHED | Integration | `crates/it/tests/db_roundtrip.rs` | `cleanup_reports_per_process_counts` |
| runtime/dispatch-run-loop: An error that ends dispatch still runs the cleanup hook and reports both errors | Integration | `crates/exa-udf-runtime/tests/dispatch.rs` | `run_error_runs_cleanup_and_reports_both_errors`, `db_close_runs_cleanup_before_relaying_the_close` |
| runtime/dispatch-run-loop: An error that ends dispatch still runs the cleanup hook and reports both errors | Unit | `crates/exa-udf-runtime/src/cleanup_tests.rs` | `fold_keeps_the_original_error_first` |
| runtime/dispatch-run-loop: An error that ends dispatch still runs the cleanup hook and reports both errors | Integration | `crates/it/tests/db_roundtrip.rs` | `run_and_cleanup_errors_both_surface` |
| runtime/dispatch-run-loop: Validation failure before dispatch skips the cleanup hook | Integration | `crates/exa-udf-runtime/tests/dispatch.rs` | `output_shape_mismatch_skips_cleanup`, `schema_mismatch_skips_cleanup` |
| runtime/dispatch-run-loop: The cleanup hook receives a CleanupContext with handshake metadata and connect-back | Unit | `crates/exa-udf-runtime/src/rowset_tests.rs` | `cleanup_context_refuses_connection_lookup` |
| runtime/dispatch-run-loop: The cleanup hook receives a CleanupContext with handshake metadata and connect-back | Integration | `crates/exa-udf-runtime/tests/dispatch.rs` | `cleanup_runs_once_after_the_last_group` |
| runtime/dispatch-run-loop: The cleanup hook receives a CleanupContext with handshake metadata and connect-back | Integration | `crates/it/tests/db_roundtrip.rs` | `cleanup_reports_per_process_counts`, `cleanup_connects_back_with_a_resolved_connection_object` |
| runtime/dispatch-run-loop: The CleanupContext refuses CONNECTION lookups without sending MT_IMPORT | Unit | `crates/exa-udf-runtime/src/rowset_tests.rs` | `cleanup_context_refuses_connection_lookup` |
| runtime/dispatch-run-loop: The CleanupContext refuses CONNECTION lookups without sending MT_IMPORT | Integration | `crates/exa-udf-runtime/tests/dispatch.rs` | `cleanup_connection_lookup_is_refused_without_mt_import` |
| runtime/dispatch-run-loop: The CleanupContext refuses CONNECTION lookups without sending MT_IMPORT | Integration | `crates/exa-udf-runtime/tests/single_call.rs` | `single_call_cleanup_runs_before_finished` |
| runtime/dispatch-run-loop: The CleanupContext refuses CONNECTION lookups without sending MT_IMPORT | Integration | `crates/it/tests/db_roundtrip.rs` | `cleanup_connects_back_with_a_resolved_connection_object` |
| runtime/dispatch-single-call: The cleanup hook runs after the single-call loop | Integration | `crates/exa-udf-runtime/tests/single_call.rs` | `single_call_cleanup_runs_before_finished`, `single_call_error_still_runs_cleanup` |
| runtime/dispatch-single-call: The cleanup hook runs after the single-call loop | Integration | `crates/it/tests/db_roundtrip.rs` | `export_into_script_fails_on_cleanup_error` |

Test assertions that carry the scenarios:

- `cleanup_runs_once_after_the_last_group` drives `CLEANUP_REPORTS` over two groups. The first client request after `MT_CLEANUP` MUST be `MT_CLOSE`, carrying `script=CLEANUP_REPORTS groups=2`, the total row count, and `io_rejected=true`.
- `successful_cleanup_precedes_mt_finished` asserts that the first client request after `MT_CLEANUP` is `MT_FINISHED` and that the runtime returns `Ok`.
- `cleanup_runs_when_no_group_ran` answers the first `MT_RUN` of `CLEANUP_AFTER_RUN_ERROR` with `MT_CLEANUP`. The next client request MUST be `MT_CLOSE` carrying `cleanup failed on purpose`.
- `run_error_runs_cleanup_and_reports_both_errors` and `run_and_cleanup_errors_both_surface` assert that `run failed on purpose` appears before `cleanup failed on purpose` in one message.
- The two `*_skips_cleanup` tests assert that the close message carries the validation error and no `cleanup failed on purpose`.
- `cleanup_connection_lookup_is_refused_without_mt_import` asserts that the first client request after `MT_CLEANUP` is `MT_CLOSE`, not `MT_IMPORT`. Its text MUST carry the `CleanupContext` refusal, which names the cleanup phase and `run()`.
- `cleanup_context_refuses_connection_lookup` builds a `CleanupContext` with a known `script_name`. It asserts that `connection("CB_SELF")` returns `UdfError::ConnectBack` naming the cleanup phase and `run()`, and that `take_last_error` returns that text. It also asserts that `script_name()` returns the handshake value and that `next`, `get`, and `emit` return errors.
- `single_call_cleanup_runs_before_finished` asserts that the first client request after `MT_CLEANUP` is `MT_CLOSE`, not `MT_IMPORT`. Its text MUST carry `cleanup ran after export_spec` and the `CleanupContext` refusal, so the single-call session's cleanup receives a `CleanupContext`, not a `SingleCallContext`.
- `cleanup_connects_back_with_a_resolved_connection_object` asserts that the statement fails with a message carrying `cleanup connect-back read 42 connection_refused=true`.
- `cleanup_reports_per_process_counts` runs `CLEANUP_REPORTS` over 8 groups of 3 rows. It parses `groups=<g> rows=<r>` from the error and asserts `1 <= g <= 8` and `r == 3 * g`. It also asserts that the error carries `script=CLEANUP_REPORTS` and `io_rejected=true`.

### Manual Testing

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| sdk/udf-abi | `cargo test -p exasol-udf-sdk abi` | `cleanup_slot_takes_context_and_abi_version_is_eleven` passes, 0 failures |
| sdk/udf-macro | `cargo test -p exasol-udf-macros` | the five `cleanup` tests and the trybuild case pass, 0 failures |
| runtime/dispatch-run-loop | `cargo test -p exa-udf-runtime --all-features --test dispatch cleanup` | 0 failures, and the list includes `cleanup_connection_lookup_is_refused_without_mt_import` |
| runtime/dispatch-run-loop | `cargo test -p exa-udf-runtime cleanup` | 0 failures without the `connect-back` feature, and the list includes `cleanup_context_refuses_connection_lookup` and `cleanup_connection_lookup_is_refused_without_mt_import` |
| runtime/dispatch-single-call | `cargo test -p exa-udf-runtime --all-features --test single_call cleanup` | 0 failures |
| cleanup-hook fixture | `cargo build --release -p cleanup-hook && cargo run -q -p cargo-exasol-udf -- exasol-udf validate target/release/libcleanup_hook.so` | `✓ 6 UDF(s) validated in 'target/release/libcleanup_hook.so'` |
| End to end | `cargo test -p it --features integration` | `[it] scenario cleanup_ok_statement_succeeds ok`, `[it] scenario cleanup_reports_per_process_counts ok`, `[it] scenario run_and_cleanup_errors_both_surface ok`, `[it] scenario cleanup_connects_back_with_a_resolved_connection_object ok`, and `[it] scenario export_into_script_fails_on_cleanup_error ok` on stderr |

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Build | `cargo build --release` | Exit 0 |
| Test | `cargo test` | 0 failures |
| Test, all features | `cargo test -p exa-udf-runtime --all-features` | 0 failures |
| Integration | `cargo test -p it --features integration` | 0 failures |
| Lint | `cargo clippy --all-targets --all-features -- -D warnings` | 0 warnings |
| Format | `cargo fmt --check` | No changes |
