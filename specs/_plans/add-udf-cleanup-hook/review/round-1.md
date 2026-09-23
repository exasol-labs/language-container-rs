# Plan Review Findings: add-udf-cleanup-hook (round 1)

## Summary
- Axes checked: 6/6
- Total findings: 14 (Blockers: 0, Advisory: 14)
- Intent Fidelity blockers: 0
- Human-escalation blockers: 0

## Premortem

1. A Python author ports an audit-row write into the Rust cleanup hook over connect-back. The invoking statement is still open while the engine waits for `MT_FINISHED`, so the write conflicts and hits `WAIT FOR COMMIT`. The engine `recv` retries without a timeout (`zmqinternal.cc` `send_or_recv`), so the statement hangs. Routed to Feasibility `[NFR_IGNORED]`.
2. A later refactor re-enters the teardown step when the `MT_FINISHED` exchange fails, so the hook runs twice. No test counts hook invocations, so CI stays green. Routed to Requirement Quality `[COMPLETENESS_GAP]` (exactly-once).
3. The IT matrix goes red on 8.29.x only. The single-call teardown error or the connect-back login during cleanup behaves differently on the older engine, and the plan names no fallback. A local `scripts/ci-it-local.sh` run fails too, because that script never builds `libcleanup_hook.so`. Routed to Feasibility `[UNSTATED_ASSUMPTION]` and `[HIDDEN_DEPENDENCY]`.

## Intent Fidelity
[no objection, axis checked: the reference client runs `vm->shutdown()` before `MT_FINISHED`, on every post-construction error including a DB `MT_CLOSE` and single-call errors, and after zero groups (`exaudflib_main.cc:63-72, 195-286`, `socket_high_level.cc:294-300`). The plan runs the hook at each of these points and skips it on pre-dispatch validation failure. It executes the interview answers as agreed: `destroy` repurposed as `Option<(ctx, error_out) -> i32>`, no outcome flag, and a dedicated `CleanupContext`. The macro shim builder, the loader helper, and the rowset macro split are each required by the hook, so no scope creep.]

## Feasibility

#### [HIDDEN_DEPENDENCY] ADVISORY
- Location: plan.md § Implementation Tasks 2.1, § Parallelization (group B Knowledge)
- Issue: Task 2.1 moves the return-code and out-pointer handling "into one loader helper that `run` and `cleanup` share". `LoadedUdf::run` is public, and `crates/exa-udf-runtime/tests/emit_arrow_dlopen.rs:72` calls it and expects `-> i32`. That test compiles only under `--features emit-arrow-test` or `--all-features`, so a featureless `cargo test` hides a break. The out-pointer reader `take_c_string` lives in `single_call.rs`, so the new helper would make `loader.rs` depend on a dispatcher module.
- Fix: In task 2.1, state that `LoadedUdf::run` keeps its `-> i32` signature and that the shared helper is a separate internal method. Move `take_c_string` from `single_call.rs` into `loader.rs`. Add `tests/emit_arrow_dlopen.rs` and `single_call.rs` to the group B Knowledge column.

#### [EFFORT_MISESTIMATION] ADVISORY
- Location: plan.md § Implementation Tasks 2.5
- Issue: `start_mock_session` in `crates/exa-udf-runtime/tests/dispatch.rs` sets `script_name = lib.to_uppercase()`. For the new fixture it resolves `__exa_udf_entry_CLEANUP_HOOK`, which none of the six entry points exports. `drive_session` accepts a script name, but it drives one group and always answers the first `MT_RUN` with `MT_RUN`. `cleanup_runs_once_after_the_last_group` (two groups), `cleanup_runs_when_no_group_ran`, and `db_close_runs_cleanup_before_relaying_the_close` fit neither helper.
- Fix: Add to task 2.5: "Give `start_mock_session` a script-name parameter separate from the fixture lib name, and use it for every `cleanup-hook` entry point."

#### [HIDDEN_DEPENDENCY] ADVISORY
- Location: plan.md § Implementation Tasks 2.4
- Issue: `scripts/ci-it-local.sh:78-104` builds its own `-p` fixture list, commented "keep in step with the CI build job". Task 2.4 wires `cleanup-hook` into `.github/workflows/ci.yml` only. A local run of that script then fails the five new IT scenarios on a missing `libcleanup_hook.so`.
- Fix: In task 2.4, add `-p cleanup-hook` to the `cargo build --release` list in `scripts/ci-it-local.sh`.

#### [UNSTATED_ASSUMPTION] ADVISORY
- Location: decision-log.md § [2] Consequences and § [3] Consequences, and `runtime/dispatch-single-call/spec.md` § Scenario: The cleanup hook runs after the single-call loop (last AND)
- Issue: Two live claims rest on engine behavior outside this repo. Decision [3] states the first: the engine accepts a connect-back login during the cleanup exchange. The second is unstated: a single-call VM's cleanup `MT_CLOSE` fails the `EXPORT ... INTO SCRIPT` statement after the hook already returned its SQL. The planner read one engine source tree, while CI runs 8.29.x, 2025.1.x, and 2026.1.x. Neither claim names a fallback if its IT fails.
- Fix: Add the single-call teardown assumption to decision [2] Consequences, naming `export_into_script_fails_on_cleanup_error` as its check. For both assumptions, state the fallback in plan.md: drop the live AND clause from the scenario and document the limit in `docs/writing-a-udf.md`.

#### [NFR_IGNORED] ADVISORY
- Location: plan.md § Implementation Tasks 2.8
- Issue: The hook runs while the invoking statement is still open, because the engine waits for `MT_FINISHED` or `MT_CLOSE`. A connect-back write in cleanup that conflicts with the invoking query hits the `WAIT FOR COMMIT` deadlock that CLAUDE.md § Connect-back describes. Task 2.8 documents only the `connection` versus `connect_back` rule.
- Fix: Add to task 2.8: document in `docs/writing-a-udf.md` §2 that the invoking statement's transaction is still open during cleanup, so connect-back writes follow the same no-conflict rule as writes during `run`.

## Requirement Quality

#### [COMPLETENESS_GAP] ADVISORY
- Location: plan.md § Verification (Scenario Coverage rows for "The cleanup hook runs once after the last group, before MT_FINISHED" and "The cleanup hook runs after the single-call loop", and the test-assertion list)
- Issue: Both scenarios require the hook to run once, and the dispatch-run-loop scenario says "exactly once". No assertion counts invocations. `cleanup_runs_once_after_the_last_group` checks groups, rows, and `io_rejected` only. `single_call_cleanup_runs_before_finished` asserts `MT_CLOSE`, so its name claims an `MT_FINISHED` ordering that it never tests.
- Fix: Make the `CLEANUP_REPORTS` hook increment a static call counter and add `calls=<n>` to its error text. Assert `calls=1` in `cleanup_runs_once_after_the_last_group` and `cleanup_reports_per_process_counts`. Rename `single_call_cleanup_runs_before_finished` to `single_call_cleanup_receives_cleanup_context` in plan.md.

#### [REQUIREMENT_CONFLICT] ADVISORY
- Location: `runtime/dispatch-run-loop/spec.md` § Scenario: Validation failure before dispatch skips the cleanup hook
- Issue: The THEN reads "it MUST NOT invoke the cleanup hook, because no group or call ran". The scenario "The cleanup hook runs once after the last group" requires the hook when no group ran. The stated cause contradicts that rule. The real cause is that dispatch never started.
- Fix: Replace "because no group or call ran" with "because dispatch never started".

#### [AMBIGUOUS_REQUIREMENT] ADVISORY
- Location: `runtime/dispatch-run-loop/spec.md` § Scenario: An error that ends dispatch still runs the cleanup hook and reports both errors, and plan.md task 2.2 (the `cleanup.rs` fold)
- Issue: The scenario fixes the order of the two texts but not how a reader tells them apart in one message. `RuntimeError::Udf` displays as `UDF error: <text>`, so a naive fold nests that prefix. `cleanup.rs` also appends the context-recorded error to the hook's own error. For `CLEANUP_CONNECTION`, which returns the refusal unchanged, the refusal text then appears twice in the message.
- Fix: State the fold format in task 2.2, for example `<original> (cleanup also failed: <cleanup>)`, and assert it exactly in `fold_keeps_the_original_error_first`. In task 2.2, skip the appended context error when the hook's text already contains it.

#### [COMPLETENESS_GAP] ADVISORY
- Location: plan.md task 2.2, and `runtime/dispatch-run-loop/spec.md` § Scenario: The cleanup hook receives a CleanupContext with handshake metadata and connect-back
- Issue: `UdfContext::input_column_count` is a required trait method, and `input_column`, `output_column_count`, and `output_column` have defaults. Neither the task nor the scenario states what `CleanupContext` returns for them. A Python `cleanup()` reads `exa.meta` columns, so a copy of `SingleCallContext`'s `0` reports no columns for a data UDF without saying so.
- Fix: In task 2.2 and the scenario, state what `CleanupContext` returns for the four column-metadata accessors: the handshake's declared columns, or `0` and `Err` as in `SingleCallContext`.

#### [REQUIREMENT_CONFLICT] ADVISORY
- Location: `specs/runtime/connect-back-query/spec.md` § Background (recorded), and `runtime/dispatch-run-loop/spec.md` § Feature description (delta)
- Issue: The recorded connect-back-query Background states that "`connection(name)` retrieves the raw credentials ... via an on-demand `MT_IMPORT` exchange", with no exception. That feature also owns the per-context connect-back scenarios for `SingleCallContext`. After this plan, `CleanupContext::connection` never sends `MT_IMPORT`, and connect-back-query does not say so. The rewritten dispatch-run-loop Feature paragraph also keeps the cross-reference `runtime/connect-back`, which names no existing feature.
- Fix: Add a DELTA:CHANGED Background to `runtime/connect-back-query` with one sentence: `CleanupContext::connection` refuses without `MT_IMPORT`, as `runtime/dispatch-run-loop` specifies. In the dispatch-run-loop delta Feature paragraph, change `runtime/connect-back` to `runtime/connect-back-query`.

#### [COMPLETENESS_GAP] ADVISORY
- Location: decision-log.md § [2] Consequences (mid-group bullet), and `sdk/udf-abi/spec.md` § Scenario: cleanup absent leaves the slot None
- Issue: Decision [2] runs the hook on a mid-group `MT_CLEANUP`, but no scenario states it and no test registers a hook on that path. `mid_group_cleanup_ends_session_cleanly` uses `scalar_double`, which has no hook. The udf-abi AND "exactly as for a UDF built before the hook existed" is false on that path, because a hookless UDF now sends `MT_FINISHED` where it sent nothing.
- Fix: Replace that udf-abi AND with "the runtime MUST answer the DB's `MT_CLEANUP` with `MT_FINISHED` without invoking a hook". In plan.md § Verification, state that the mid-group hook path stays untested because the engine never sends a mid-group `MT_CLEANUP`.

## Task Breakdown

#### [TASK_GRANULARITY] ADVISORY
- Location: plan.md § Implementation Tasks 2.2
- Issue: Task 2.2 builds two modules as one unit: `CleanupContext` plus the macro split in `rowset.rs`, and the new `cleanup.rs` with its fold. It carries two test files in 13 sentences. A failure in either half blocks verification of the other.
- Fix: Split task 2.2 into 2.2 (`CleanupContext`, macro split, `cleanup_context_refuses_connection_lookup`) and a new 2.3 (`cleanup.rs`, `fold_keeps_the_original_error_first`). Renumber the later tasks and the "tasks 2.4 to 2.7" reference above the fixture table.

## Design Depth

ADR gate: decisions [1] (vtable slot and ABI), [2] (teardown owner and hook timing), and [3] (one `UdfContext` type per protocol phase) each change architecture or design, so `Promotes to ADR: yes` passes. Decision [4] is `no`. The `rowset.rs` macro split is consistent: `HostContextBridge` and `SingleCallContext` invoke both halves, and `CleanupContext` invokes only the `cluster_ip` plus `connect_back` half with its own ungated `connection`.

#### [INFORMATION_LEAKAGE] ADVISORY
- Location: plan.md § Design (the `cleanup` module paragraph), and task 2.1
- Issue: `dispatch::invoke_run` and `single_call::invoke_ctx_hook` each build the double-indirection context pointer (`&mut &mut dyn UdfContext` cast to `*mut c_void`). The plan adds a third copy in `cleanup.rs`. Task 2.1 centralizes only the return-code handling. A change to the context-pointer contract then needs three edits.
- Fix: In task 2.1, make the shared internal loader helper take `&mut dyn UdfContext` and build the pointer itself, for both `run` and `cleanup`. Keep the public `LoadedUdf::run` unchanged for its external test caller.

## Prose Quality

#### [PROSE_BLOAT] ADVISORY
- Location: decision-log.md § [3] Alternatives and Consequences, and `sdk/udf-abi/spec.md` § Scenario: The cleanup slot replaces destroy in place, bumping the ABI version
- Issue: The user asked that decisions and specs read as the current design, "not a changelog". Decision [3] Consequences narrates a revision: "The last interview answer replaces the earlier answer to reuse `SingleCallContext` as-is." Its Alternatives list the chosen option: "A dedicated `CleanupContext` type: chosen." The udf-abi scenario frames the slot as a migration ("WHEN the vtable is compiled under this change").
- Fix: Delete both decision [3] lines. Rephrase the udf-abi scenario as a current contract: the slot after `run` is `cleanup: Option<...>` with the `run` signature, and `EXA_UDF_ABI_VERSION` is 11, so a `.so` built against ABI 10 fails with `AbiMismatch`. Update the matching Scenario Coverage row in plan.md.
