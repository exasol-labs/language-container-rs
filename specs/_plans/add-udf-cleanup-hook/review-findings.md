# Code Review Findings: add-udf-cleanup-hook

## Summary
- Files reviewed: 32
- Total findings: 9 (standard: 8, expert: 1)
- Evidence at review time: `cargo clippy --all-targets --all-features -- -D warnings` clean, `cargo fmt --check` clean, `cargo test -p exa-udf-runtime [--all-features] cleanup` 21 passed with and without features, `cargo test -p exasol-udf-macros --test cleanup` 5 passed
- Checked with no finding: the shared shim builder maps `Ok`/`Err`/panic to `0`/`1`/`2` and guards a null `error_out`. `call_lifecycle_slot` frees the out-pointer exactly once, keeps the exact `UDF <slot> returned error code <rc>[: <text>]` text, and allocates nothing when `rc == 0`. Neither dispatcher sends `MT_FINISHED` any more, and `Runtime::run` sends exactly one final message per dispatch exit, including the mid-group `MT_CLEANUP` exit. Both pre-dispatch validation sites skip the hook. `CleanupContext::connection` refuses without touching the control channel.

## Standard fixes

### crates/exa-udf-runtime/src/cleanup.rs

#### [SENTINEL_ERROR_VALUE] The hook outcome is an inverted `Option<RuntimeError>` instead of a `Result`
- Location: lines 22-30 (`invoke_hook`), lines 41-56 (`fold`)
- Issue: `invoke_hook` returns `Option<RuntimeError>`, where `None` means success. `fold` then takes the dispatch outcome as `Result<(), RuntimeError>` and the cleanup outcome as `Option<RuntimeError>`, so one signature uses two encodings for the same idea, a step that may have failed. The guardrail requires the language's error mechanism for failure, not an in-band absent value. The tests reflect the mismatch: `fold(Err(..), Some(..))` and `fold(Ok(()), None)`.
- Fix: In crates/exa-udf-runtime/src/cleanup.rs, change `invoke_hook` to return `Result<(), RuntimeError>`. Bind `let Some(result) = udf.cleanup(&mut ctx) else { return Ok(()) };`, then return `result.map_err(|e| <recorded-detail helper>(e, ctx.take_last_error()))`, where the helper is `with_recorded_detail` (or its replacement from the Expert finding below). Change `fold`'s second parameter to `cleanup: Result<(), RuntimeError>` and write its body as `match (outcome, cleanup) { (outcome, Ok(())) => outcome, (Ok(()), Err(c)) => Err(c), (Err(o), Err(c)) => Err(RuntimeError::Udf(format!("{} (cleanup also failed: {})", message_of(&o), message_of(&c)))) }`. In crates/exa-udf-runtime/src/cleanup_tests.rs, update every `fold` call to pass `Ok(())` in place of `None` and `Err(..)` in place of `Some(..)`, keep the asserted strings unchanged, and run `cargo test -p exa-udf-runtime cleanup::tests`.

### crates/exasol-udf-macros/src/lib.rs

#### [SHRINKABLE] Four identical `section(path)` parse arms
- Location: lines 80-99 (`impl Parse for Annotations::parse`, arms `vs_adapter`, `import_spec`, `export_spec`, `cleanup`)
- Issue: the new `cleanup` arm is the fourth copy of `let content; syn::parenthesized!(content in input); annotations.<field> = Some(content.parse::<Path>()?);`. The Rule of Three says to extract it.
- Fix: In crates/exasol-udf-macros/src/lib.rs, add a private free function `fn parse_path_section(input: ParseStream) -> syn::Result<Path> { let content; syn::parenthesized!(content in input); content.parse() }` next to `parse_schema_fields`. Replace each of the four arm bodies with a single line such as `annotations.cleanup = Some(parse_path_section(input)?);`, using each arm's own field. Run `cargo test -p exasol-udf-macros` and confirm that `unknown_annotation_section.stderr` and the five `cleanup` tests still pass.

### crates/exa-udf-runtime/tests/dispatch.rs

#### [SHRINKABLE] The test inlines the `expect_close` helper defined in the same file
- Location: `cleanup_connection_lookup_is_refused_without_mt_import`, the block from `let req = recv_req(&server);` through `.expect("close carries an exception message");` (around lines 1515-1530)
- Issue: the block repeats the body of `expect_close`, which receives the request, asserts `MtClose`, and extracts `exception_message`. It also adds an `assert_ne!(.., MtImport)` that the following `assert_eq!(.., MtClose)` already implies.
- Fix: In crates/exa-udf-runtime/tests/dispatch.rs, replace that block in `cleanup_connection_lookup_is_refused_without_mt_import` with `let msg = expect_close(&server, "the DB accepts no MT_IMPORT after MT_CLEANUP, so the refused lookup must close the session");`. Leave the following `msg.contains(..)` assertion and the `client.join()` check as they are. Run `cargo test -p exa-udf-runtime --test dispatch cleanup_connection_lookup`.

### crates/exasol-udf-sdk/src/abi_tests.rs

#### [INLINE_COMMENT] Inline comment explains the test through the removed `destroy` slot
- Location: lines 130-131 (`cleanup_slot_takes_context_and_abi_version_is_eleven`)
- Issue: `// Assigning this function pins the run slot's (ctx, error_out) -> i32 shape on cleanup: the no-argument destroy shape fails to compile here.` explains the test by pointing to a slot that no longer exists once this change merges. The `cleanup: Some(fail_with_ctx_presence)` assignment and the test name already state the pinned shape.
- Fix: In crates/exasol-udf-sdk/src/abi_tests.rs, delete the two comment lines above `unsafe extern "C" fn fail_with_ctx_presence` in `cleanup_slot_takes_context_and_abi_version_is_eleven`, and change nothing else.

### crates/it/tests/db_roundtrip.rs

#### [REDUNDANT_COMMENT] Section comment restates the scenario calls that follow
- Location: line 326 (`db_roundtrip_all_scenarios`)
- Issue: `// Cleanup-hook scenarios.` repeats what the five `cleanup_*`/`export_into_script_fails_on_cleanup_error` calls and their `eprintln!` lines already say.
- Fix: In crates/it/tests/db_roundtrip.rs, delete the line `// Cleanup-hook scenarios.` in `db_roundtrip_all_scenarios`.

### docs/writing-a-udf.md

#### [OUTDATED_COMMENT] Cleanup timing and error-reporting prose contradicts itself and the runtime
- Location: lines 185, 189, 193 (section "Session-end cleanup: the `cleanup(path)` section")
- Issue: Line 185 says the hook runs "when the database sends `MT_CLEANUP`", but the second bullet lists a `run()` error and a database close, and the database sends no `MT_CLEANUP` in either case. Line 189 says the hook is skipped on a validation failure "because no group ran", which contradicts line 187 ("It also runs when that process ran no group at all"). Line 193 says a panic "fails the statement the same way", right after a sentence promising the hook's error text. A panic carries only `UDF cleanup returned error code 2` (`call_lifecycle_slot` with `rc == 2` and a null out-pointer).
- Fix: In docs/writing-a-udf.md: (1) on line 185, replace "when the database sends `MT_CLEANUP`:" with "when that process's dispatch ends:". (2) On line 189, replace ", because no group ran." with ": the session closes with the validation error alone." (3) On line 193, replace "A panic in the hook is caught and fails the statement the same way." with "A panic in the hook is caught and fails the statement with `F-UDF-CL-RUST-9001` and `UDF cleanup returned error code 2`, without hook text."

### docs/protocol.md

#### [OUTDATED_COMMENT] Cleanup step omits the mid-group `MT_CLEANUP` that now also ends through the hook and `MT_FINISHED`
- Location: line 88 ("**3. Cleanup.**")
- Issue: The paragraph names only `MT_RUN` and `MT_DONE` answered with `MT_CLEANUP`. `run_group` also ends the session when a mid-group `MT_NEXT` is answered with `MT_CLEANUP` (`GroupExit::Session`). With this change, that exit runs the hook and sends `MT_FINISHED`, as the updated `mid_group_cleanup_ends_session_cleanly` asserts.
- Fix: In docs/protocol.md line 88, replace "When the DB answers an `MT_RUN` or an `MT_DONE` with `MT_CLEANUP`," with "When the DB answers an `MT_RUN`, an `MT_DONE`, or a mid-group `MT_NEXT` with `MT_CLEANUP`,".

### specs/architecture.md

#### [OUTDATED_COMMENT] Lifecycle line attaches `exit(0)` to the error close and ties every teardown to `MT_CLEANUP`
- Location: lines 66-68 (the **Lifecycle:** bullet, `cleanup` clause)
- Issue: The clause reads "`cleanup` (on MT_CLEANUP, …, then MT_FINISHED, or the error close when dispatch or the hook failed, then `exit(0)`)". After an error close, `exaudfclient` exits with `Exit { code, .. }` (crates/exaudfclient/src/main.rs:41-44), not `exit(0)`. A dispatch error also reaches this step without any `MT_CLEANUP`.
- Fix: In specs/architecture.md, replace the `cleanup` clause of the **Lifecycle:** bullet (lines 66-68) with: "`cleanup` (once the run loop or the single-call loop ends: the UDF's optional `cleanup(path)` hook runs once with a `CleanupContext`, then MT_FINISHED and `exit(0)`, or one error MT_CLOSE and a non-zero exit when dispatch or the hook failed)."

## Expert fixes

### crates/exa-udf-runtime/src/cleanup.rs, crates/exa-udf-runtime/src/single_call.rs

#### [INFORMATION_LEAKAGE] Two modules merge a context-recorded error into a hook error, in two different formats
- Location: crates/exa-udf-runtime/src/cleanup.rs lines 32-39 (`with_recorded_detail`); crates/exa-udf-runtime/src/single_call.rs lines 234-237 (`invoke_ctx_hook`, the `r.map_err(|e| match bridge.take_last_error() { .. })` closure)
- Issue: One decision, how a hook's error absorbs the detail its context recorded, now lives in two modules. `cleanup::with_recorded_detail` appends the raw text of `RuntimeError::Udf` and skips a detail the hook text already contains. `single_call::invoke_ctx_hook` formats `format!("{e}: {detail}")` from the whole `RuntimeError`, so the result reads `UDF error: UDF error: <hook text>: <detail>` with a doubled prefix, and it repeats a detail the hook already reported. The next change to that format would have to edit both modules, and nothing keeps them in step.
- Fix: (1) In crates/exa-udf-runtime/src/error.rs, add `impl RuntimeError { pub(crate) fn with_recorded_detail(self, detail: Option<String>) -> RuntimeError { .. } }` with the exact body of `cleanup::with_recorded_detail`. Give it a doc comment saying it appends a context-recorded error to a hook's `Udf` error unless the hook text already contains it. (2) Delete `with_recorded_detail` from crates/exa-udf-runtime/src/cleanup.rs and call `e.with_recorded_detail(ctx.take_last_error())` in `invoke_hook`. (3) In crates/exa-udf-runtime/src/single_call.rs `invoke_ctx_hook`, replace the `map_err` closure body with `e.with_recorded_detail(bridge.take_last_error())`. (4) Move the tests `recorded_detail_is_appended_when_the_hook_text_lacks_it`, `recorded_detail_is_not_repeated_when_the_hook_already_reported_it`, and `hook_error_without_recorded_detail_is_unchanged` from crates/exa-udf-runtime/src/cleanup_tests.rs into a new crates/exa-udf-runtime/src/error_tests.rs. Declare it as the last item of error.rs with `#[cfg(test)] #[path = "error_tests.rs"] mod tests;` and rewrite the calls in method form. (5) Run `cargo test -p exa-udf-runtime --all-features` and confirm that `adapter_connection_probe_combines_hook_and_recorded_errors`, `single_call_cleanup_runs_before_finished`, and `cleanup_connection_lookup_is_refused_without_mt_import` still pass.
