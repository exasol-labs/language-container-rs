# Plan Review Findings: add-arm64-support (round 2)

## Summary
- Axes checked: 6/6
- Total findings: 4 (Blockers: 0, Advisory: 4)
- Intent Fidelity blockers: 0

## Premortem

Three ways the revised plan still fails six months out:

1. **P5 regresses the production x86_64 install.** Task 5.1 refactors `install.sh` to source a shared helper described as owning "existing-entry preservation." But `install.sh` never preserved entries — it overwrites `SCRIPT_LANGUAGES` wholesale. A helper built to preserve, then sourced by `install.sh`, silently changes the shipped x86_64 path's behavior. → Feasibility / Design Depth.
2. **P3 reds the x86_64 integration leg it swears to protect.** Task 3.1 renames `build-slc`'s artifact per arch, but the `integration` job downloads it by the fixed name `lc-tarball`; no task updates that consumer, so the x86_64 IT leg (decision [6]: "IT stays x86_64") cannot find its tarball. → Feasibility.
3. **A superseded interview task creeps back in.** The decision-log Interview answer still lists "keeping the branch current" and "narrowing the PR description" as in-scope tasks, contradicting the authoritative decision [10] that dropped both. → Requirement Quality.

## Round-1 Blocker Recheck

- **Resolved: [HIDDEN_DEPENDENCY] P0 execution model undefined.** The fresh-in-repo-branch model is now stated end-to-end and coherently: plan.md Summary ("applying PR #51's arch-neutral four-file change onto a fresh in-repo branch off `main`"), the Architecture diagram ("P0: fresh in-repo branch off main + apply PR #51's arch-neutral 4-file change"), the § Decision, the § Dependencies bullet, and an explicit § P0 "Branch model" note ("Implementation runs on a fresh in-repo branch off `main` … NOT on PR #51 … The arch-neutral four-file change is therefore NOT inherited"). Task **0.1** applies PR #51's four-file diff; **0.2** is the version bump; **0.3** the fail-fast guards; there is **no task 0.0 and no task 0.4**. Decision **[10]** is rewritten to the fresh-branch model and records that the branch-currency and "narrow the PR #51 description" tasks are dropped. A `grep` for `0.0` / `0.4` / `already present` / `narrow` / `currency` finds no orphaned active references — the only "narrow"/"currency" mentions are inside decision [10]'s Alternatives and the `[plan-review]` entry, both correct historical records. Confirmed resolved, not merely reworded.

All eight round-1 advisories are also confirmed resolved:
1. **Stale-data-layout claim** — decision [4], § Dead Code Removal, and § Consequences now rest the §7 removal solely on "no in-repo rustc consumer / sole `COPY targets/` reference"; the only staleness mention is decision [4]'s explicit negation ("not stale — byte-for-byte identical"). Resolved.
2. **P4 ordering** — task 4.3 now fixes concrete order (P3/P4 land first; #67/#69 rebase onto the arm matrix). Resolved.
3. **Version-bump serialization** — § Cross-cutting release hygiene adds the serialize-on-landing paragraph. Resolved.
4. **Fail-fast MUST** — Scenario Coverage row (line 185, task 0.3) + Manual Testing entry (line 204). Resolved.
5. **cargo-exaudf ignored rows** — three integration rows annotated `#[ignore]`d (lines 180–182) + Checklist `--ignored` step (line 215). Resolved.
6. **personal-install rows** — both rows now Unit + Manual + restart-idempotency Manual entry (line 207). Resolved.
7. **P5 5.1 split** — now 5.2 (reading + assembly + `ALTER SYSTEM`) and 5.3 (transport + extraction + reconciliation). Resolved (see new advisory on 5.1's wording).
8. **#-fragment shared helper** — task 5.1 extracts the fragment into `scripts/lib/script_languages.sh`, sourced by both scripts; 5.4 tests point at it. The `#`-fragment invariant now has a single owner. Resolved (see new advisory on the preservation over-claim).

## Intent Fidelity

[no objection — axis checked: all §0–§7 sections still map to phases P0–P7 with none dropped or added; spec-delta scope is unchanged and correct (§1 cargo-exaudf, §2 crate-license-notices, §5 personal-install, §0 slim-image — CHANGED titles verified to match the recorded library character-for-character, NEW features confirmed absent from the library); all five preserved platform facts remain encoded — PT_INTERP loader-placement (slim-image scenario + decision [8]a), glibc-not-musl `exaudfclient` (crate-license-notices + [8]b), `#`-fragment executable/no-leading-slash (personal-install + [8]c), SSH-port-read-fresh (personal-install + [8]d), IT-stays-x86_64 (decision [6]/[8]e). No INTENT-clean round-1 item re-raised.]

## Feasibility

#### [HIDDEN_DEPENDENCY] ADVISORY
- Location: plan.md § P3 task 3.1 (and § Parallelization "independently-landable" claim)
- Issue: Task 3.1 makes `build-slc` a matrix and uploads "the per-arch tarball as distinctly-named artifacts," but the current `build-slc` uploads a single fixed-name artifact `lc-tarball` (`.github/workflows/ci.yml:288–292`) that **two** downstream jobs download by that exact name: `integration` (ci.yml:354–357) and `release` (ci.yml:534–538). Task 3.2 handles the `release` job's collection of both tarballs, but **no task updates the `integration` job's download**. Renaming the artifact per arch breaks the x86_64 `integration` leg — the leg decision [6] requires to keep passing — so P3 is not as self-contained as "independently-landable" claims; it reds CI as written.
- Fix: In plan.md task 3.1, add that the matrix rename requires updating the `integration` job's artifact-download step (`.github/workflows/ci.yml` ~354) to fetch the x86_64-named artifact, and name the `integration` job (not only `release`) as an affected consumer of the renamed `lc-tarball`.

#### [UNSTATED_ASSUMPTION] ADVISORY
- Location: plan.md § P5 task 5.1; decision-log.md [7]/[8]c
- Issue: Task 5.1 says to "Extract the `SCRIPT_LANGUAGES` `#`-fragment and registration-string assembly (executable path, no leading slash, **existing-entry preservation**) out of the inline expression at `scripts/install.sh:123`." `install.sh:123` assigns `SCRIPT_LANGUAGES="RUST=…#buckets/…/exaudf/exaudfclient"` wholesale — a single `RUST=` value with **no read of the current parameter and no append**. There is no existing-entry-preservation logic in `install.sh` to extract; preservation is net-new behavior the personal-install scenario ("preserves existing entries") requires only on the `ALTER SYSTEM` path. Additionally `install.sh`'s statement is scope-parameterized (`ALTER ${SCOPE_UPPER}`, default `SESSION`), not fixed `SESSION`. Building preservation into the shared helper and then sourcing it from `install.sh` risks silently changing the shipped x86_64 path from overwrite to merge.
- Fix: In plan.md task 5.1, scope the shared helper to the `#`-fragment / registration-string **assembly** only (executable path, no leading slash), and state that existing-entry preservation is new logic added in task 5.2 for `install-personal.sh`'s `ALTER SYSTEM` path — not an extraction from `install.sh`. Add that the `install.sh` refactor MUST preserve its current single-value `ALTER ${SCOPE}` overwrite behavior.

## Requirement Quality

#### [REQUIREMENT_CONFLICT] ADVISORY
- Location: decision-log.md § Interview (line 9)
- Issue: The Interview answer still records — as the planner's own summary, not verbatim user words — that "keeping the branch current with main … narrowing the PR description … are in scope as tasks." Decision [10] explicitly supersedes both ("the conditional-rebase task is dropped, and the 'narrow the PR #51 description' task is dropped"). The Interview section now contradicts the authoritative decision and the task list; an implementer skimming it could reintroduce a dropped task.
- Fix: In decision-log.md § Interview, annotate the line-9 answer that "keeping the branch current with main" and "narrowing the PR description" were superseded and dropped by decision [10], or strike those two clauses.

#### [COMPLETENESS_GAP] ADVISORY
- Location: specs/_plans/add-arm64-support/container/crate-license-notices/spec.md § "Target set reflects the shipped glibc binary"; plan.md § Scenario Coverage
- Issue: The scenario's third normative clause — "a comment in `about.toml` MUST record why the shipped binary is glibc and why both libc/arch triples are listed" — has no mapped verification. The Scenario Coverage row maps only `about_toml_lists_gnu_triples` (asserts the gnu triples are present), which does not assert the comment. This is the same MUST-without-a-test pattern round-1 flagged for the fail-fast guard, left unaddressed for this clause.
- Fix: In plan.md § Scenario Coverage, either extend the `about_toml_lists_gnu_triples` assertion (or add a sibling assertion) to check the required `about.toml` comment is present, or add a "code review" verification row for the comment clause referencing task 2.1.

## Task Breakdown

[no objection on traceability — axis checked: every delta scenario maps to an implementing task (cargo-exaudf → P1.1–1.5; slim-image → P0.1/0.3 + PR #51's applied builder change; crate-license-notices → P2.1–2.2; personal-install → 5.1–5.4) and every task implements in-scope work; the P5 split into 5.1 (helper) / 5.2 (assembly + `ALTER SYSTEM`) / 5.3 (transport) / 5.4 (tests) gives each concern a separately-verifiable unit. Granularity concern raised under Feasibility for task 3.1's hidden second edit rather than duplicated here.]

## Design Depth

[no objection on the core design — axis checked: the shared `scripts/lib/script_languages.sh` gives the `22002`-crash-if-wrong `#`-fragment invariant a single owner, resolving the round-1 information-leakage finding; parameterizing one pipeline by host-derived arch remains the deep, drift-resistant choice. The one residual boundary nuance — the helper's responsibility line between fragment assembly and entry preservation — is captured in the § Feasibility [UNSTATED_ASSUMPTION] advisory rather than repeated here.]

## Prose Quality

[no objection — axis checked: plan.md Summary holds the two-sentence cap and leads with the outcome; phase headings are statement-style; the § P0 Branch-model note is long but unambiguous and load-bearing; spec-delta feature-description lines are single-sentence and active-voice. No filler or ambiguous pronoun blocks action. The decision-log Interview inconsistency is a correctness contradiction, raised under Requirement Quality, not a prose defect.]
