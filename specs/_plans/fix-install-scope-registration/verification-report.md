# Verification Report: fix-install-scope-registration

## Verdict

| Result | Details |
|--------|---------|
| **PASS** | Registration consolidated into one `register_script_languages` function called by both transports; `--scope` removed; every path now registers with `ALTER SYSTEM SET SCRIPT_LANGUAGES` after a read-merge step. No `[workspace.package].version` bump: only `scripts/install.sh`, its test, and docs changed, no compiled artifact. |
| Code review | 7 findings — 7 fixed (6 standard, 1 expert) |

| Check | Status |
|-------|--------|
| Build | ✓ |
| Tests | ✓ |
| Lint | ✓ |
| Format | ✓ |
| Scenario Coverage | ✓ |
| Manual Tests | ✓ |

## Test Evidence

### Coverage

| Type | Coverage % |
|------|------------|
| Unit | Not measured (shell suite has no coverage tool; Rust workspace unaffected by this plan) |
| Integration | N/A — no Rust production code changed |

### Test Results

| Type | Run | Passed | Ignored |
|------|-----|--------|---------|
| Shell unit (`install-personal-test.sh`) | 115 | 115 | 0 |
| Rust (`cargo test`, full workspace, unaffected by this plan) | 372 | 370 | 2 |

### Manual Tests

| Test | Result |
|------|--------|
| Default path, entry preservation: live docker-db (`exasol/docker-db:2026.1.1`) pre-loaded with `R=builtin_r JAVA=builtin_java PYTHON3=builtin_python3`; ran `scripts/install.sh --host localhost --password exasol --bfs-password <pw> --skip-build` | ✓ — `SCRIPT_LANGUAGES` became `R=builtin_r JAVA=builtin_java PYTHON3=builtin_python3 RUST=...`; before this fix, `--scope system` would have dropped everything but `RUST` |
| Default path, registration persists: re-queried `EXA_PARAMETERS` from a separate `exapump sql` invocation (a new session) | ✓ — `RUST` still present, `ALTER SYSTEM` persisted it |
| Default path, flag removed: `scripts/install.sh --host localhost --password exasol --bfs-password <pw> --scope system` | ✓ — failed with `Unknown option: --scope`, printed usage |
| Default path, idempotent: re-ran the first command | ✓ — one `RUST` entry after the second run, not two |
| Registration banner names the correct endpoint | ✓ — printed `Registering RUST at localhost:8563 (ALTER SYSTEM SET SCRIPT_LANGUAGES)`, confirming finding 4.7's fix |
| container/personal-install-cloud (live cloud Personal) | Not run — no cloud Personal deployment available in this session; covered by unit tests `registers_by_merging_into_alter_system` and `both_transports_call_the_shared_registration`, which pin that the HTTP arm (the arm cloud Personal also uses) reaches the shared function |
| container/personal-install (live local Personal, scalar UDF invocation) | Not run — no local Personal deployment available in this session; this plan does not change local-Personal behavior, and its existing unit-tested paths are untouched |

## Tool Evidence

### Linter

```
shellcheck -x scripts/install.sh              → 0 warnings
shellcheck -x scripts/tests/install-personal-test.sh → 0 warnings
cargo clippy --all-targets --all-features -- -D warnings → 0 warnings (full workspace)
```

### Formatter

```
cargo fmt --check → no changes
bash -n scripts/install.sh / scripts/tests/install-personal-test.sh → syntax OK
```

## Scenario Coverage

| Domain | Feature | Scenario | Test Location | Test Name | Passes |
|--------|---------|----------|---------------|-----------|--------|
| container | personal-install | Registration is system-scoped and preserves existing entries | `scripts/tests/install-personal-test.sh` | `registers_by_merging_into_alter_system`, `both_transports_call_the_shared_registration`, `preserves_existing_script_languages` | Pass |
| container | personal-install | Registration refuses a SCRIPT_LANGUAGES value it could not read | `scripts/tests/install-personal-test.sh` | `refuses_to_alter_when_the_current_value_cannot_be_read`, `refuses_to_read_a_current_value_the_query_cannot_supply` | Pass |
| container | personal-install-cloud | Cloud install uses the standard HTTP transport | `scripts/tests/install-personal-test.sh` | `registers_by_merging_into_alter_system`, `both_transports_call_the_shared_registration` | Pass |

## Notes

Code review raised 7 findings (6 standard, 1 expert); all 7 were fixed and re-verified (115/115 shell assertions green after the fixes). The expert finding (4.7) closed a wrong-endpoint defect in the registration banner: it now derives `host:port` from the `dsn` argument itself via a new `dsn_endpoint` helper, tested against a password containing `@`, a password containing `?`, and a dsn with no `@` — no ordering of those inputs can leak credential text into the banner.

Two items surfaced during review are out of this plan's scope and are not fixed here:
- `current_script_languages` (the function immediately above `register_script_languages`) still prints its own read-failure error from the `$HOST`/`$PORT` globals rather than deriving the endpoint from its `dsn` argument the way 4.7 fixed the banner one function below. Same defect class (wrong endpoint on a multi-target run), not a credential leak. Not named in the review findings, so left untouched; a one-token fix (`$HOST:$PORT` → `$(dsn_endpoint "$dsn")`) if it needs to be picked up.
- `plan.md`'s own narrative text (not its Scenario Coverage table, which is corrected) still describes the pre-4.5-rename test name in two prose sentences describing task 1; the Scenario Coverage table and the actual test file agree on the current name `refuses_to_read_a_current_value_the_query_cannot_supply`.

No `[workspace.package].version` bump: this plan's decision log records a direction change (Design Decision [2]) reversing the plan's original version-bump call. `scripts/install.sh` is deployment tooling, not the compiled runtime/SDK/container image that `CLAUDE.md`'s version-bump rule protects, so the version stays at `0.28.1` and downstream UDF builds are unaffected. `git diff --stat Cargo.toml Cargo.lock` confirms no change.

Manual verification ran against a live `exasol/docker-db:2026.1.1` container standing in for the default (no `--deployment`) path, using a minimal stub tarball via `SLC_TARBALL` (registration behavior does not depend on the SLC binary's contents). Cloud Personal and local Personal manual rows were not exercised — no such deployment was available in this session — and rely on the unit-test coverage above plus the fact that this plan changes no local-Personal code path.
