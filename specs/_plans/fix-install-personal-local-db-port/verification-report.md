# Verification Report: fix-install-personal-local-db-port

## Verdict

| Result | Details |
|--------|---------|
| **PASS** | `resolve_local_connection` resolves the local SQL port from `connection.dbPort` (honoring `--port`), all automated checks are green, and the review round found 5 issues, all fixed. Manual live-deployment testing was not executed (no live multi-deployment Personal environment in this run) — see Notes. |
| Code review | 5 findings — standard: 5, expert: 0 — all 5 fixed |

| Check | Status |
|-------|--------|
| Build | ✓ |
| Tests | ✓ |
| Lint | ✓ |
| Format | ✓ |
| Scenario Coverage | ✓ (unit-covered scenarios); manual live scenarios not executed |
| Manual Tests | ✗ (not executed — no live Personal deployments available in this environment) |

## Test Evidence

### Coverage

| Type | Coverage % |
|------|------------|
| Unit | Not measured (no `cargo llvm-cov` run this pass); 3 new install-script assertions added, all cargo unit/doc test suites unaffected and green |
| Integration | Excluded by default (`it` crate requires `--features integration` + live Docker DB; out of scope for this change, which touches only `scripts/install.sh` and its shell test harness) |

### Test Results

| Type | Run | Passed | Ignored |
|------|-----|--------|---------|
| Shell (`install-personal-test.sh`) | 1 file, all `run`-registered assertions | All (`All assertions passed.`) | 0 |
| Cargo (`cargo test`) | 55 test binaries/doc-test groups | 305 | 0 |

### Manual Tests

| Test | Result |
|------|--------|
| Local, non-default port (`--deployment agent-alpha`) registers on its own DB | Not executed — needs two live local Personal deployments on different SQL ports |
| Local, other deployment untouched | Not executed — same dependency |
| Local, `--port` honored (fails against the overridden port) | Not executed — same dependency |
| Local, default preserved (`--deployment default`, no `connection.dbPort`) | Not executed — same dependency |
| UDF executes on the registered deployment | Not executed — same dependency |

## Tool Evidence

### Linter

```
$ shellcheck scripts/install.sh scripts/tests/install-personal-test.sh   (via docker run --rm koalaman/shellcheck:stable)
Baseline unchanged: 3x SC1091 (info, on `source` lines) + 3x SC2034 (warning, CLI_PORT/CLI_USER/BFS_PASSWORD in the test file) — no new warnings introduced by this change.

$ cargo clippy --all-targets --all-features -- -D warnings
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 2.52s
(0 warnings)
```

### Formatter

```
$ cargo fmt --check
(exit 0, no diff)
```

## Scenario Coverage

| Domain | Feature | Scenario | Test Location | Test Name | Passes |
|--------|---------|----------|---------------|-----------|--------|
| container | personal-install | Local connection details resolve from the deployment directory (`dbPort` present, never cached, missing descriptor) | `scripts/tests/install-personal-test.sh` | `resolves_local_connection_from_descriptor` | Pass |
| container | personal-install | `--port` overrides the resolved local port | `scripts/tests/install-personal-test.sh` | `cli_port_overrides_local_descriptor` | Pass |
| container | personal-install | Local port falls back to `PERSONAL_DB_PORT_DEFAULT` (8563) when `connection.dbPort` is absent | `scripts/tests/install-personal-test.sh` | `resolves_local_defaults_when_db_port_absent` | Pass |
| container | personal-install | `HOST` stays fixed at the local forwarder regardless of descriptor content or `--host` | `scripts/tests/install-personal-test.sh` | `resolves_local_connection_from_descriptor` (post-fix: descriptor host ≠ forwarder host) | Pass |
| container | personal-install | Connection details are read fresh on every run (SSH port: existing regression guard) | `scripts/tests/install-personal-test.sh` | `reads_ssh_port_from_deployment_json` | Pass |
| container | personal-install | A registered Rust UDF executes on Personal (live, non-`8563` port) | Manual, live local Personal deployment | — | Not executed |

The remaining nine pre-existing scenarios (`fragment_points_at_executable_no_leading_slash`, `preserves_existing_script_languages`, `parses_current_script_languages_from_query_output`, `selects_transport_from_backend`, the five `cloud_*`/`resolves_cloud_*` tests) are untouched by this plan and still pass, confirmed by the full `install-personal-test.sh` run (all assertions passed) and unaffected `cargo test`.

## Notes

- **Manual live testing not executed.** The plan's Manual Testing section requires two live local Exasol Personal deployments on different SQL ports (e.g. `default`:8563, `agent-alpha`:52164). This headless implementation run has no such environment. All logic the manual steps would exercise is covered by the three new unit tests (`resolve_local_connection`'s descriptor read, `--port` override, and default fallback) plus the pre-existing `resolve_cloud_connection`/SSH-port tests, which use the identical `deployment_field` seam. Recommend running the plan's Manual Testing table against a real multi-deployment host before/shortly after merge.
- **Code review**: 5 standard findings, 0 expert findings, all fixed by a follow-up `implementer-agent` pass (Phase 4 in `tasks.md`). Findings: a test-boundary gap where the descriptor host equaled the forwarder host (couldn't distinguish "HOST hardcoded correctly" from "HOST accidentally read from the descriptor"); an outdated header comment claiming a restart reassigns "both" ports (only SSH does); a doc-comment gap on `resolve_local_connection`'s output contract; a stray magic-number `'8563'` literal left in `resolve_cloud_connection` after the constant rename; and an ambiguous "that port" reference in `docs/installation.md` once two ports appear in the same row.
- No dead code introduced or left behind: `PERSONAL_DB_PORT` was renamed (not duplicated) to `PERSONAL_DB_PORT_DEFAULT`, with all three references (declaration, `usage()` interpolation, `resolve_cloud_connection`'s default) updated consistently.
- No non-`--deployment` invocation, cloud branch, or unrelated local-branch logic (password resolution, `SCOPE=SYSTEM`, SSH port, node-key check) was touched, matching the plan's Non-Goals.
