# Verification Report: fix-install-personal-local-db-port

This report supersedes the plan's prior verification-report.md, which covered only tasks 1-4 (the original port fix). This round covers tasks 5-11 (Phase 5, groups D-I): unifying `resolve_cloud_connection`/`resolve_local_connection` into `resolve_deployment_connection`, per the PR #85 review comment.

## Verdict

| Result | Details |
|--------|---------|
| **PASS** | Two resolvers unified into one; all nine review findings (3 standard, 3 expert) fixed and re-verified; automated suite green. |
| Code review | 6 findings — standard: 3, expert: 3 — 6 fixed |

| Check | Status |
|-------|--------|
| Build | ✓ |
| Tests | ✓ |
| Lint (clippy) | ✓ |
| Format | ✓ |
| Shellcheck | ⚠ not run — not installed in this sandbox, no passwordless sudo to install it; needs CI or a manual run before merge |
| Scenario Coverage | ✓ |
| Manual Tests | — not run this round (unchanged from prior verification-report.md; requires two live Personal deployments — see plan.md § Manual Testing) |

## Test Evidence

### Test Results

| Type | Run | Passed | Ignored |
|------|-----|--------|---------|
| Shell unit (`install-personal-test.sh`) | 57 | 57 | 0 |
| Cargo workspace (`cargo test`) | all workspace crates | all passed (0 failed) | 0 |

Shell suite grew from 51 assertions (pre-review) to 57 (post-review): +2 from finding [MISSING_BOUNDARY_TEST] (cloud host-default boundary), +2 from finding [SWALLOWED_ERROR] (unreadable-descriptor guard, TDD red→green), +1 from finding [MISSING_BOUNDARY_TEST] (`require_cloud_bfs_password` success path), -1 from finding [DUPLICATE_TEST] (removed tautological assertion), and 2 existing assertions hardened in place (no count change) by finding [ASSERTION_FREE_TEST]'s sentinels.

### Manual Tests

Not exercised this round — no live Personal deployment available in this environment. The prior round's manual-testing plan (`plan.md` § Verification § Manual Testing) still applies unchanged: two local Personal deployments on different SQL ports, confirming the printed `Registering RUST at <host>:<port>` banner and a successful `ALTER SYSTEM` against the non-default port.

## Tool Evidence

### Linter (clippy)

```
$ cargo clippy --all-targets --all-features -- -D warnings
exit 0, no warnings
```

Shellcheck could not be run in this sandbox (binary not installed, `sudo apt-get install` requires a password not available here). `bash -n` was used as a syntax substitute on every edit throughout implementation and passed each time.

### Formatter

```
$ cargo fmt --check
exit 0, no diff
```

### Build

```
$ cargo build --release
exit 0
```

## Scenario Coverage

Per `plan.md` § Verification § Scenario Coverage (this revision's table), cross-referenced against the shell suite's 57 passing assertions:

| Scenario | Test Location | Test Name | Passes |
|----------|---------------|-----------|--------|
| Local connection details resolve from the deployment directory | `scripts/tests/install-personal-test.sh` | `resolves_local_connection_from_descriptor`, `resolves_local_host_default_when_absent`, `resolves_local_user_from_descriptor`, `resolves_local_defaults_when_db_port_absent` | Pass |
| Command-line flags override descriptor-derived local values | `scripts/tests/install-personal-test.sh` | `cli_host_overrides_local_descriptor`, `cli_port_overrides_local_descriptor`, `cli_flags_override_cloud_descriptor` | Pass |
| A local descriptor that omits the SQL port is reported | `scripts/tests/install-personal-test.sh` (unreadable descriptor); manual (live warning) | `resolves_local_connection_from_descriptor` (third resolve) | Pass (unit half) |
| Cloud connection details resolve from the deployment directory | `scripts/tests/install-personal-test.sh` | `resolves_cloud_connection_from_descriptor` (now sentinel-guarded), `resolves_cloud_defaults_when_connection_fields_absent` | Pass |
| Cloud has no host default (new, review-driven) | `scripts/tests/install-personal-test.sh` | `cloud_requires_host_when_descriptor_omits_it` | Pass |
| Cloud install requires an operator-supplied BucketFS password | `scripts/tests/install-personal-test.sh` | `cloud_requires_operator_bfs_password` (now asserts both the failure and the success path) | Pass |
| Cloud install fails when no DB password resolves | `scripts/tests/install-personal-test.sh` | `cloud_requires_db_password` | Pass |
| Cloud install uses the standard HTTP transport and scope | `scripts/tests/install-personal-test.sh`; manual | `cloud_leaves_scope_untouched` | Pass (unit half) |
| An unreadable deployment descriptor fails resolution outright (new, review-driven) | `scripts/tests/install-personal-test.sh` | `unreadable_descriptor_fails_even_with_cli_overrides` | Pass |
| Registration is system-scoped and preserves existing entries | `scripts/tests/install-personal-test.sh`; manual (printed endpoint) | `preserves_existing_script_languages` | Pass (unit half) |
| A registered Rust UDF executes on Personal | Manual only | — | Not run this round |

Three behaviors remain manual-only, as recorded in `plan.md`: the missing-`dbPort` warning, the printed `${HOST}:${PORT}` endpoint, and `main`'s wiring to the new functions — all live in `main()`, which the sourced harness cannot execute.

## Review Findings Disposition

All 6 code-review findings from `review-findings.md` were fixed and re-verified against the finding's own mutation/probe before being marked resolved:

| Finding | Category | Fixed via |
|---------|----------|-----------|
| `require_cloud_bfs_password` success path unasserted | Standard / MISSING_BOUNDARY_TEST | tasks.md 5b.4 |
| Five dead `BFS_PASSWORD="bfspw"` assignments | Standard / UNUSED_VARIABLE | tasks.md 5b.5 |
| Tautological second `HOST` assertion | Standard / DUPLICATE_TEST | tasks.md 5b.6 |
| Cloud's empty `default_host` argument unasserted | Expert / MISSING_BOUNDARY_TEST | tasks.md 5b.1 |
| `resolves_cloud_connection_from_descriptor` PORT/USER assertions vacuous | Expert / ASSERTION_FREE_TEST | tasks.md 5b.2 |
| Unreadable descriptor silently yields empty PORT/USER | Expert / SWALLOWED_ERROR | tasks.md 5b.3 |

## Notes

- **Shellcheck is the one unresolved checklist item.** It is not installed in this sandbox and there is no passwordless `sudo` to install it. It last ran clean against this file family in the prior round (tasks.md 3.2); CI or a manual run before merge should confirm it stays clean against this round's edits.
- **Design decisions and their rationale** for the unification (superseding decisions [2] and [5], resolving the deferred half of [8]) are in `decision-log.md` under the entries appended by this revision.
- **Historical references outside scope:** `specs/_decision/025-add-arm64-support.md` still names `resolve_cloud_connection` in prose — it is a recorded historical artifact outside this plan's Features table, not a live reference, and the code reviewer explicitly excluded it from findings.
