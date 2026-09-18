# Plan: fix-install-scope-registration

## Summary

`scripts/install.sh` registers `SCRIPT_LANGUAGES` twice, and only the local-Personal copy reads and merges the current value first (issue #115). Consolidate registration into one `register_script_languages` function that every transport calls, drop `--scope`, and always run `ALTER SYSTEM SET SCRIPT_LANGUAGES` after the read-merge step.

## Context

`main()` branches on `LOCAL_TRANSPORT`. The local-Personal branch reads the current `SCRIPT_LANGUAGES` value, merges the `RUST` entry into it, and refuses to register when that read fails. PR #113 added that behavior for issue #110. The other branch serves two callers: the default path with no `--deployment` (docker-db, an on-prem cluster, SaaS) and `--deployment` naming a cloud-backend Personal deployment. That branch assigns the `RUST` entry alone and runs `ALTER ${SCOPE_UPPER} SET SCRIPT_LANGUAGES`. At `--scope system` the statement replaces the instance's persistent value with the `RUST` entry alone, dropping every other registered language.

The split is the defect. One decision, how to write `SCRIPT_LANGUAGES`, lives in two places, and the two copies disagree. Transport is the only genuine difference between the branches: SSH copy, shared-directory extract, or BucketFS HTTP upload. `exasol-labs/lakehouse-engine-rs` installs the same SLC and already branches on transport alone, with one shared registration function.

`register_script_languages` takes a DSN and the assembled `RUST` entry. It owns the read, the merge, the banner, and the statement. That interface is narrower than the four-step block it replaces at each call site, and no caller needs to know how the merge works, so a later change to the merge rule stays inside one function.

`--scope` is what made the second copy look reasonable. Its `SESSION` default also made the default install weaker than it reads: `ALTER SESSION` applies to the session that issues it, and `exapump sql` opens one session per invocation, so the registration does not outlive the install command. Removing the flag removes both the trap at `SYSTEM` and the ineffective default.

## Features

| Feature | Status | Spec |
|---------|--------|------|
| container/personal-install | CHANGED | `container/personal-install/spec.md` |
| container/personal-install-cloud | CHANGED | `container/personal-install-cloud/spec.md` |

The default path with no `--deployment` gets the same registration fix in code and documentation. It carries no spec file today and gains none here.

## Impact

Breaking: `--scope` is removed. Any invocation that passes `--scope SESSION` or `--scope SYSTEM` now fails with `Unknown option: --scope`. Callers drop the flag; no replacement flag exists, because every install now registers with `ALTER SYSTEM`.

Operators running `scripts/install.sh --scope system` against a database with other registered languages get the fix: those entries survive. Operators running the default `SESSION` scope get a registration that survives the install command, which the previous per-invocation session scope did not.

New requirement: the DB user given to the install now needs `SELECT` on `EXA_PARAMETERS` on every path, not only on local Personal. The install fails with a clear error when the read does not return a value, instead of registering `RUST` as the only language. `sys` and an Exasol administrator account already hold that grant.

Local Personal behavior is unchanged. Cloud Personal changes from `ALTER SESSION` with no merge to `ALTER SYSTEM` with the merge.

This drops a documented CLI flag, so it is observable by downstream operators. It touches only `scripts/install.sh`, an operator-facing deployment script, not the compiled artifacts `CLAUDE.md`'s version-bump rule protects: the runtime binary, the SDK/macro surface, or the container image. None of those change, so `[workspace.package].version` stays at `0.28.1` and downstream UDFs need no rebuild.

## Dependencies

None. `current_script_languages`, `parse_script_languages`, `script_languages_with_rust_entry`, and `csv_unquote` already exist in `scripts/install.sh` and are already unit-tested.

## Implementation Tasks

1. Replace the scope assertions in `scripts/tests/install-personal-test.sh` with assertions on the shared registration function, before that function exists. Delete `cloud_leaves_scope_untouched` (`:463-477`) and its `run` line (`:768`), and delete `SCOPE=SESSION` from `reset_connection_globals` (`:303`): both pin the behavior this plan removes. Add `registers_by_merging_into_alter_system`, following the `PATH` stub pattern of `refuses_to_read_a_current_value_the_query_cannot_supply` (`:274-283`). Its `exapump` stub appends `"$*"` to `$stub_dir/calls` and, when the arguments contain `SELECT`, prints `CURRENT_SCRIPT_LANGUAGES` then `PYTHON3=builtin_python3 JAVA=builtin_java`. Call `register_script_languages "exasol://sys:x@127.0.0.1:8563" "$PERSONAL_ENTRY"`, then assert the recorded call log holds exactly one line matching `ALTER SYSTEM SET SCRIPT_LANGUAGES='PYTHON3=builtin_python3 JAVA=builtin_java ${PERSONAL_ENTRY}'`. The fixture value must carry two pre-existing aliases, because a single-alias fixture cannot distinguish a merge from an overwrite. Add `refuses_to_alter_when_the_current_value_cannot_be_read`, whose stub exits `1` on the `SELECT`: assert `register_script_languages` returns `1` and that the call log contains no `ALTER`. Add `both_transports_call_the_shared_registration`, a structural assertion on `main`'s own body. Assert `check "both transport arms call the shared registration" "2" "$(declare -f main | grep -c register_script_languages)"`. Assert `check "no arm issues its own registration statement" "0" "$(declare -f main | grep -c 'SET SCRIPT_LANGUAGES')"`. `declare -f main` prints `main` alone, so the banner and the statement inside `register_script_languages` do not raise the second count. That second count is `4` against the current script, so the assertion fails before task 2 lands. This test is the guard against a future arm dropping the shared call or issuing its own `ALTER`. Register all three tests in the `run` list beside `refuses_to_read_a_current_value_the_query_cannot_supply` (`:760`).
2. Add `register_script_languages <dsn> <entry>` to `scripts/install.sh` beside the existing merge helpers, and call it from both arms of step 3. The function reads the current value through `current_script_languages`, returns `1` when that read fails, merges through `script_languages_with_rust_entry`, prints `==> Registering RUST at ${HOST}:${PORT} (ALTER SYSTEM SET SCRIPT_LANGUAGES) …`, and runs `exapump sql "ALTER SYSTEM SET SCRIPT_LANGUAGES='<merged>'" -d "$dsn"`. It MUST `return`, never `exit`, so the sourced test harness drives it under `set +e`, matching the seam `resolve_deployment_connection` already uses. Its doc comment states the design intent: registration is transport-independent, so one function owns how `SCRIPT_LANGUAGES` is written and every transport calls it. In the local arm (`:692-698`) keep the `ENTRY=` assignment and replace the read, merge, banner and `exapump sql` lines with `register_script_languages "$DSN" "$ENTRY" || die "cannot register the RUST language"`. In the HTTP arm (`:717-723`) rename `SCRIPT_LANGUAGES=` to `ENTRY=`, keep the `DSN=` assignment, and replace the banner and `exapump sql` lines with the same call; its closing `echo "    ${SCRIPT_LANGUAGES}"` becomes `echo "    ${ENTRY}"`, which is what the label `SCRIPT_LANGUAGES entry` already claims. Rename the section header `# ── Personal: SCRIPT_LANGUAGES value handling ──` (`:435`), because the helpers under it now serve every transport.
3. Remove the `--scope` surface from `scripts/install.sh`. Delete the `SCOPE=SESSION` default (`:69`), the `--scope)` arg-loop case (`:566`), the `SCOPE=SYSTEM` override in the local-Personal branch (`:619`), and the `SCOPE_UPPER` assignment and validation block with its bash-3.2 comment (`:641-646`). Delete the `--scope SESSION|SYSTEM` option line from `usage()` (`:138`) and the `--scope SYSTEM` argument from the SaaS example (`:159`). In `usage()`, state that every install registers with `ALTER SYSTEM SET SCRIPT_LANGUAGES` and preserves the languages already registered. Run `shellcheck scripts/install.sh` after this task: tasks 2 and 3 together are what leave no unused variable behind.
4. Update `docs/installation.md`. Replace the cloud paragraph (`:180-184`), which says `--scope` behaves as it does without `--deployment` and that the cloud path does not force `SYSTEM`: state instead that the cloud path registers with `ALTER SYSTEM SET SCRIPT_LANGUAGES` after merging into the current value, identically to the local path, and keep the `--host`/`--port`/`--user`/`--password` override sentence. Add one sentence to the automated-install section (after the option-reference line, `:53`) stating that the install reads the current `SCRIPT_LANGUAGES` value, adds the `RUST` entry beside the entries already registered, and persists the result with `ALTER SYSTEM`. Leave the manual-install section unchanged: `:318` and `:321` offer an operator a hand-run `ALTER SESSION` or `ALTER SYSTEM` choice in the operator's own session, which this plan does not touch, and `:343` names `scripts/install.sh` only as an alternative to re-running that statement, without claiming a scope flag.
5. Sync `specs/mission.md` with the new registration contract. At `:22` and `:31`, change `ALTER SESSION SET SCRIPT_LANGUAGES` to `ALTER SYSTEM SET SCRIPT_LANGUAGES`. At `:43`, restate the glossary row as the general Exasol mechanism: an SLC is registered with a `SET SCRIPT_LANGUAGES` statement at `ALTER SESSION` or `ALTER SYSTEM` scope. That row defines the SLC concept, not this install script, and both scopes stay valid Exasol statements. Change no other mission text.
No task carries `[expert]`. The merge helpers, the sourced-harness seam, and the `PATH` stub pattern all exist and are tested; this work rewires three call sites, deletes one flag, and syncs two documents.

## Parallelization

| Group | Tasks | Depends on | Knowledge |
|-------|-------|------------|-----------|
| A: registration consolidation | 1, 2, 3, 4, 5 | — | spec deltas `container/personal-install`, `container/personal-install-cloud`; `scripts/install.sh`, `scripts/tests/install-personal-test.sh`, `docs/installation.md`, `specs/mission.md` |

Group A is one cluster, not five. Tasks 1 to 3 edit two files that both describe the same registration contract. Tasks 4 and 5 state that same contract, for operators and for the mission. Splitting the documentation out would give a second agent the same contract to re-derive.

## Dead Code Removal

| Type | Location | Reason |
|------|----------|--------|
| Global | `scripts/install.sh:69` `SCOPE=SESSION` | The flag it backed is removed; every install registers with `ALTER SYSTEM` |
| Arg-loop case | `scripts/install.sh:566` `--scope)` | Same |
| Assignment | `scripts/install.sh:619` `SCOPE=SYSTEM` | A forced override of a variable that no longer exists |
| Validation block | `scripts/install.sh:640-646` `SCOPE_UPPER` and its `die` | Nothing reads `SCOPE_UPPER` once both arms call `register_script_languages` |
| Test | `scripts/tests/install-personal-test.sh:463-477` `cloud_leaves_scope_untouched` | Asserts that cloud resolution leaves `SCOPE` at `SESSION`, which is the defect |
| Assignment | `scripts/tests/install-personal-test.sh:303` `SCOPE=SESSION` in `reset_connection_globals` | Resets a variable the script no longer declares |

`parse_script_languages`, `csv_unquote`, `script_languages_with_rust_entry`, and `current_script_languages` all survive. `register_script_languages` composes them, and their own unit tests stay valid.

## Verification

### Scenario Coverage

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| Registration is system-scoped and preserves existing entries | Unit + Manual | `scripts/tests/install-personal-test.sh`; live docker-db and live Personal | `registers_by_merging_into_alter_system`, `both_transports_call_the_shared_registration`, `preserves_existing_script_languages`; see Manual Testing |
| Registration refuses a SCRIPT_LANGUAGES value it could not read | Unit | `scripts/tests/install-personal-test.sh` | `refuses_to_alter_when_the_current_value_cannot_be_read`, `refuses_to_read_a_current_value_the_query_cannot_supply` |
| Cloud install uses the standard HTTP transport | Unit + Manual | `scripts/tests/install-personal-test.sh`; live cloud Personal deployment | `registers_by_merging_into_alter_system`, `both_transports_call_the_shared_registration`; see Manual Testing |

Two tests together cover the registration half of all three scenarios. `registers_by_merging_into_alter_system` calls `register_script_languages` directly and pins what that function does. `both_transports_call_the_shared_registration` reads `main`'s body and pins that both arms reach that function and issue no registration statement of their own. Issue #115 is the case where one arm skipped the shared behavior, so the second test is the guard against its return. The transport-specific half of the cloud scenario, the `exapump bucketfs cp` upload, is unchanged and stays manual, as it was before this plan.

The remaining scenarios in both features are untouched and stay covered as recorded: `fragment_points_at_executable_no_leading_slash`, `parses_current_script_languages_from_query_output`, `selects_transport_from_backend`, the connection-resolution tests, the local-mechanism tests, and manual end-to-end verification on live deployments.

The default path with no `--deployment` has no scenario, by the interview decision. Both unit tests still pin its registration, because it runs the HTTP arm that they cover, and the docker-db row below verifies it end to end.

### Manual Testing

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| default path, entry preservation | Against a docker-db carrying `PYTHON3` and `JAVA` entries: `SLC_TARBALL=lc-rs.tar.gz scripts/install.sh --host localhost --password exasol --bfs-password <pw> --skip-build`, then `exapump sql "SELECT SYSTEM_VALUE FROM EXA_PARAMETERS WHERE PARAMETER_NAME = 'SCRIPT_LANGUAGES'" -d "exasol://sys:exasol@localhost:8563?validateservercertificate=0"` | The query shows `PYTHON3`, `JAVA` and `RUST`. Before this change the same run at `--scope system` left `RUST` alone |
| default path, registration persists | Open a new session and re-run the query above | `RUST` is still present, because `ALTER SYSTEM` persisted it |
| default path, flag removed | `scripts/install.sh --host localhost --password exasol --bfs-password <pw> --scope system` | Fails with `Unknown option: --scope` and prints the usage text |
| default path, idempotent | Re-run the first command | The query shows one `RUST` entry, not two |
| container/personal-install-cloud | `SLC_TARBALL=lc-rs.tar.gz scripts/install.sh --deployment my-cloud-db --bfs-password <pw> --skip-build` | Uploads, prints `Registering RUST at <host>:<port> (ALTER SYSTEM SET SCRIPT_LANGUAGES)`, and the query above against that deployment shows `RUST` beside its pre-existing entries |
| container/personal-install (local, unchanged) | `SLC_TARBALL=lc-rs.tar.gz scripts/install.sh --deployment my-db --skip-build` | Succeeds exactly as before, then a scalar Rust UDF created over that deployment's SQL port returns the expected result |

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Install-script tests | `bash scripts/tests/install-personal-test.sh` | `All assertions passed.` (exit 0) |
| Shellcheck | `shellcheck scripts/install.sh scripts/tests/install-personal-test.sh` | No new warnings against the pre-change baseline |
| Build | `cargo build --release` | Exit 0 |
| Test | `cargo test` | 0 failures |
| Lint | `cargo clippy --all-targets --all-features -- -D warnings` | 0 warnings |
| Format | `cargo fmt --check` | No changes |
| Version unchanged | `git diff --stat Cargo.toml Cargo.lock` | No output: `scripts/install.sh` is deployment tooling, not language-container production code, so `[workspace.package].version` stays at `0.28.1` |
