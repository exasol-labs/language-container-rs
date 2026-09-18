# Plan Review Findings: fix-install-scope-registration (round 1)

## Summary
- Axes checked: 6/6
- Total findings: 8 (Blockers: 2, Advisory: 6)
- Intent Fidelity blockers: 0

## Intent Fidelity

[no objection — axis checked: plan.md § Summary and decision-log.md `[1]` implement the fix the issue body decided (one shared `register_script_languages`, `--scope` deleted, always `ALTER SYSTEM`). Both interview answers hold: no spec file or scenario exists for the default path (plan.md:24, :90), no `## Design` section exists in plan.md, and `Promotes to ADR: no` is set. Task 5 (version bump) is named in the stated intent, so it is not creep. The one boundary stretch checked and accepted: the `DELTA:NEW` background in `container/personal-install/spec.md:18` asserts "The install script exposes no scope option", a property of the whole script rather than of the Personal feature. That is the terse way to record the flag removal without the new spec file the interview refused.]

## Feasibility

#### [HIDDEN_DEPENDENCY] ADVISORY
- Location: plan.md § Impact, paragraph "New requirement: the DB user given to the install now needs `SELECT` on `EXA_PARAMETERS` on every path"
- Issue: the Impact section names the wrong new prerequisite and omits the larger one. `ALTER SYSTEM` needs an admin account, stated by this repo at `docs/installation.md:320` (`-- persists across sessions (requires admin)`). Before this plan the default path ran `ALTER SESSION`, which needs no privilege, so this change makes an admin account mandatory for every install. The `SELECT` on `EXA_PARAMETERS` claim carries no evidence, and `docs/installation.md:303` tells any operator to run that same query with no privilege caveat.
- Fix: In plan.md § Impact, replace the `SELECT` on `EXA_PARAMETERS` sentence with the admin requirement, citing `docs/installation.md:320`. State that every install now needs an account that may run `ALTER SYSTEM`, where the default path previously accepted a non-admin account at `SESSION` scope. In task 4, extend the sentence added after `docs/installation.md:53` to name the admin requirement for the automated install.

#### [UNSTATED_ASSUMPTION] ADVISORY
- Location: plan.md § Verification § Manual Testing; `container/personal-install/spec.md:65-72` (scenario "Registration refuses a SCRIPT_LANGUAGES value it could not read")
- Issue: the plan assumes `SELECT SYSTEM_VALUE FROM EXA_PARAMETERS WHERE PARAMETER_NAME = 'SCRIPT_LANGUAGES'` returns a non-empty data line on every target the default path serves. `parse_script_languages` (`scripts/install.sh:450-474`) treats an empty or absent data line as a hard failure, so a target that returns NULL or no row turns a previously succeeding install into a refusal. Only local Personal has ever run this read. The Manual Testing table covers docker-db and cloud Personal. It covers no on-prem cluster and no SaaS target, although `scripts/install.sh:158-159` advertises SaaS as a supported case today.
- Fix: Add one Manual Testing row that runs the read query alone against a SaaS or on-prem target and records whether `SYSTEM_VALUE` returns a value. Alternatively, state the assumption in plan.md § Impact and name SaaS and on-prem as unverified.

## Requirement Quality

#### [REQUIREMENT_CONFLICT] BLOCKER
- Location: `specs/mission.md:31` and `specs/mission.md:22` against `container/personal-install/spec.md:18` and `:51`
- Issue: `specs/mission.md:31` reads "packaged as a BucketFS tarball and registered with `ALTER SESSION SET SCRIPT_LANGUAGES`; `scripts/install.sh` builds, uploads, and registers in one step". `specs/mission.md:22` gives the DBA workflow as "Build + upload the container, `ALTER SESSION SET SCRIPT_LANGUAGES`, create scripts". The delta asserts the opposite: "The install script exposes no scope option, so no invocation registers at `SESSION` scope" and "it MUST use `ALTER SYSTEM SET SCRIPT_LANGUAGES`, the only registration statement the install script offers". No task in plan.md touches `specs/mission.md`, so recording this plan leaves the mission stating the reverse of a MUST in the library it heads.
- Fix: Add a task to plan.md § Implementation Tasks that changes `ALTER SESSION SET SCRIPT_LANGUAGES` to `ALTER SYSTEM SET SCRIPT_LANGUAGES` in `specs/mission.md:31` and `specs/mission.md:22`. Leave the glossary row at `specs/mission.md:43` alone, or restate it as the general Exasol mechanism rather than the install script's statement. Add the task to Parallelization group A. Change no other mission text.

#### [REQUIREMENT_CONFLICT] ADVISORY
- Location: `specs/_decision/025-add-arm64-support.md:142`, ADR `personal-cloud-reuses-http-transport`, § Decision; against decision-log.md § Design Decisions `[1]` Rationale
- Issue: that ADR's Decision states "`--scope` keeps its default `SESSION`; no `SYSTEM` scope is forced and no entry-preservation read-merge-write applies", and its Status stays `Accepted`. This plan reverses both clauses. The decision-log rationale narrates the supersession, but the decision log is a plan artifact that archives under `specs/_recorded/`. A reader of `specs/_decision/025` sees an accepted decision asserting the current behavior's opposite with no pointer. The planner considered this and chose to leave the file untouched, so the residual risk is recorded here rather than overruled.
- Fix: Add a task to plan.md § Implementation Tasks that appends one line under the ADR's `**Status:** Accepted` field at `specs/_decision/025-add-arm64-support.md`, reading `Superseded by plan fix-install-scope-registration: registration is transport-independent and always ALTER SYSTEM.` Change no other ADR text. If the planner keeps the file untouched instead, state that choice as one sentence in plan.md § Impact so a human reviewer decides it, not the decision log alone.

## Task Breakdown

#### [TRACEABILITY_GAP] BLOCKER
- Location: plan.md § Verification § Scenario Coverage, the paragraph at :86, and task 1
- Issue: the plan claims "`registers_by_merging_into_alter_system` covers all three scenarios because `register_script_languages` is the single call site every transport reaches". The function is one implementation with two call sites (`scripts/install.sh:692-698` and `:717-723`). The new test calls the function directly, so it passes whether or not the HTTP arm calls it. Issue #115 exists precisely because one arm skipped the shared behavior, and `.github/workflows/ci.yml:163` runs this test file alone and never runs `scripts/install.sh`. The plan therefore leaves the fixed defect class with no automated guard, while the coverage table maps the cloud scenario's "MUST register exactly as `container/personal-install`'s scenario requires" to that test.
- Fix: In task 1, add one structural assertion to `scripts/tests/install-personal-test.sh` that both arms reach the shared function, for example `check "both transport arms call the shared registration" "2" "$(declare -f main | grep -c register_script_languages)"` plus `check "no arm issues its own ALTER" "0" "$(declare -f main | grep -c 'ALTER SYSTEM SET SCRIPT_LANGUAGES')"`. Add the assertion to the scenario-coverage rows for "Registration is system-scoped and preserves existing entries" and "Cloud install uses the standard HTTP transport". Replace the "single call site" sentence at plan.md:86 with the claim the tests actually support.

#### [COMPLETENESS_GAP] ADVISORY
- Location: plan.md task 3; `scripts/install.sh:365` and `:158`
- Issue: task 3 enumerates every `--scope` deletion site except two prose sites that go stale. `scripts/install.sh:365` reads "Everything that is not a connection field (the local ALTER scope, the missing-dbPort warning, the cloud BucketFS password) stays at the call site", and no local ALTER scope survives this change. `scripts/install.sh:158` captions the SaaS example "SaaS / enterprise, persist across sessions", which stops distinguishing that example once every install persists. Task 3 deletes the `--scope SYSTEM` argument on the next line but leaves the caption.
- Fix: In plan.md task 3, add the deletion of "the local ALTER scope" from the comment at `scripts/install.sh:365`, and the rewrite of the example caption at `scripts/install.sh:158` to name the target rather than the scope, for example `# SaaS / enterprise:`.

## Design Depth

#### [INFORMATION_LEAKAGE] ADVISORY
- Location: plan.md task 2, the `register_script_languages <dsn> <entry>` signature and its banner
- Issue: the function takes the DSN as a parameter but prints `${HOST}:${PORT}` from globals, and `current_script_languages` (`scripts/install.sh:498-508`) reads the same two globals for its error text. The endpoint is therefore stated twice through two channels, one argument and two globals, with nothing enforcing agreement. In the new unit test both globals are empty (`reset_connection_globals` sets `HOST=""` at `scripts/tests/install-personal-test.sh:296`), so the banner prints an empty pair. The delta clause "it MUST print the resolved `host:port` it is registering against" (`container/personal-install/spec.md:52`) then holds only under the manual rows.
- Fix: In plan.md task 2, give `register_script_languages` the endpoint it prints, as `register_script_languages <dsn> <endpoint> <entry>` with both call sites passing `"${HOST}:${PORT}"`, or state in task 2 that the banner reads the globals on purpose and that the unit test asserts the banner text using globals the test sets itself.

## Prose Quality

#### [PROSE_BLOAT] ADVISORY
- Location: plan.md:13, :28, :50; decision-log.md:15, :17
- Issue: five governed sentences break `/speq:writing-guardrails`. Semicolons are banned and appear at plan.md:28 ("Callers drop the flag; no replacement flag exists"), plan.md:50 ("all exist and are tested; this work rewires"), decision-log.md:15 ("Transport stays branched; registration does not") and decision-log.md:17 ("historical record of the arm64 and Personal work; the active spec library"). plan.md:13 runs 31 words over three joined ideas ("That interface is narrower ... and no caller needs to know ... so a later change ... stays inside one function"). Tasks 1 and 2 are single paragraphs of more than ten sentences, several over the 20-word procedural cap, for example the `exapump` stub sentence in task 1.
- Fix: Replace each semicolon with a period. Split plan.md:13 into two sentences. Split tasks 1 and 2 into short numbered sub-steps, one instruction per line, adding no new words. Do not expand the prose: the content is dense, and only the sentence boundaries are wrong.
