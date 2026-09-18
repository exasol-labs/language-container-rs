# Decision Log: fix-install-scope-registration

## Interview

**Q:** The default (no `--deployment`) HTTP install path has never had its own `spec.md`; only the two Personal features spec the scope and merge behavior. This fix makes the default path's registration identical to Personal's. Should the default path get its own spec coverage now, or stay spec-free?
**A:** Keep it spec-free. Touch only the two spec files that already assert Personal-specific scope behavior, `container/personal-install` and `container/personal-install-cloud`. The default path's registration fix is code and documentation only, in `scripts/install.sh` and `docs/installation.md`. Add no new spec file and no new scenario for the default path.

**Q:** This fix consolidates registration into one shared function used by every transport, matching a pattern the sibling repository `lakehouse-engine-rs` already uses. Should that be logged as a plain decision-log fix entry, or promoted to a full ADR?
**A:** Plain fix entry only. No ADR. This restores the read-merge-preserve behavior the local-Personal branch already had after PR #113 and issue #110. It is not a new lasting architectural rule. Add no `## Design` section to `plan.md`, and promote nothing from this plan to an ADR.

## Design Decisions

### [1] One shared registration function, no `--scope`, always `ALTER SYSTEM`

- **Decision:** Fix issue #115 by factoring the read-merge-register step into one `register_script_languages` function in `scripts/install.sh` that every transport calls, and by removing `--scope`. The bug: the HTTP-transport arm, used by both the default path and cloud Personal, assigned the `RUST` entry with no read of the current value, so `--scope system` dropped every other registered language. Only the local-Personal arm read and merged first. Transport stays branched; registration does not.
- **Alternatives:** Copy the read-merge block into the HTTP arm and keep `--scope`. Rejected: it leaves one decision written twice, which is the defect, and keeps a `SESSION` default that does not outlive the install command.
- **Rationale:** `scripts/install.sh` is the documented operator-facing install command, so removing `--scope` is downstream-observable. This supersedes the consequence recorded in ADR `personal-cloud-reuses-http-transport` (`specs/_decision/025-add-arm64-support.md`), which states that the cloud path keeps `--scope` at `SESSION` and applies no entry preservation. That ADR stays as the historical record of the arm64 and Personal work; the active spec library is what this plan changes.
- **Promotes to ADR:** no

### [2] No `[workspace.package].version` bump

- **Decision:** Removing `--scope` from `scripts/install.sh` does not bump `[workspace.package].version`. Task 6 (bumping `0.28.1` to `0.29.0`) is dropped.
- **Alternatives:** Bump per `CLAUDE.md`'s version-bump rule, as the plan originally reasoned during review (see Design Decision [1]'s earlier rationale). Rejected at implementation time: `CLAUDE.md` ties the version to the ABI fingerprint that forces downstream UDF rebuilds, guarding the compiled runtime binary, the SDK/macro surface, and the container image. `scripts/install.sh` is deployment tooling that runs on the operator's machine before any UDF call; it changes none of those artifacts, so a bump would force UDF rebuilds for a change that cannot affect them.
- **Rationale:** Treating every documented-flag removal as bump-worthy would bump the version for tooling and docs changes too, which `CLAUDE.md` explicitly excludes. The two adversarial review rounds recorded in Review Findings [1]-[3] did not challenge the bump; this correction is a narrower reading of the same rule, made explicit here because it reverses a plan.md line that survived both rounds unchallenged.
- **Promotes to ADR:** no

## Review Findings

### [1] [plan-review] The mission still stated `ALTER SESSION` registration

- **Finding:** Round 1 raised `[REQUIREMENT_CONFLICT]` as a blocker. `specs/mission.md:22` and `:31` name `ALTER SESSION SET SCRIPT_LANGUAGES` as what `scripts/install.sh` runs. The spec deltas assert the opposite as a MUST. No task touched the mission, so recording this plan would leave the mission stating the reverse of the new behavior.
- **Direction change:** Added task 5. It changes both lines to `ALTER SYSTEM SET SCRIPT_LANGUAGES`. It also restates the SLC glossary row (`:43`) as the general Exasol mechanism, valid at either scope, because that row defines the SLC concept rather than this install script. Task 5 joins Parallelization group A, which already owns the registration contract. The version bump moved from task 5 to task 6. No other mission text changes.
- **Promotes to ADR:** no

### [2] [plan-review] No test proved that both transport arms reach the shared function

- **Finding:** Round 1 raised `[TRACEABILITY_GAP]` as a blocker. The plan claimed `register_script_languages` is the single call site every transport reaches. It is one function with two call sites. `registers_by_merging_into_alter_system` calls the function directly, so it passes whether or not the HTTP arm calls it. Issue #115 is that exact defect, and CI runs the test file alone, never `scripts/install.sh`.
- **Direction change:** Added `both_transports_call_the_shared_registration` to task 1. It greps `declare -f main` for two `register_script_languages` call lines and zero `SET SCRIPT_LANGUAGES` lines. `declare -f main` prints `main` alone, so the shared function's own banner and statement do not raise the second count. The current script scores `4` on that second grep, so the test fails before task 2 lands. The plan lists the test in the two affected Scenario Coverage rows. It replaces the single-call-site claim with what the two tests prove.
- **Promotes to ADR:** no

### [3] [plan-review] A renamed cloud scenario was marked `DELTA:CHANGED`

- **Finding:** Round 2 raised `[REQUIREMENT_CONFLICT]` as a blocker. The cloud delta renamed `### Scenario: Cloud install uses the standard HTTP transport and scope` to `### Scenario: Cloud install uses the standard HTTP transport` while keeping the `DELTA:CHANGED` marker. `/speq:spec-merge` matches `DELTA:CHANGED` by scenario name, so no recorded scenario matches and the stale one survives the merge, asserting the `--scope` honoring and `SESSION` default this plan removes. Validation does not catch it: two differently named scenarios are both well-formed.
- **Direction change:** Split the block in `specs/_plans/fix-install-scope-registration/container/personal-install-cloud/spec.md` in two, following `specs/_recorded/007-change-slc-runtime-debian/container/os-license-notices/spec.md:44-60`. A `DELTA:REMOVED` block carries the old heading with its recorded bullets verbatim; the renamed block now carries `DELTA:NEW` with its heading and bullets unchanged. No other delta text changed and no task was added: the rename is a recording-time concern, not an implementation step.
- **Promotes to ADR:** no
