# Plan Review Findings: fix-db-mem-size-noop (round 1)

## Summary
- Axes checked: 6/6
- Total findings: 7 (Blockers: 2, Advisory: 5)
- Intent Fidelity blockers: 0
- Human-escalation blockers: 0

## Premortem

Six months from now this plan failed. Three ways it could have happened:

1. The implementer runs task 2.1. The grep prints two `binary file matches` lines besides the `AGENTS.md` bullet. One comes from the stale, gitignored `it-runner` binary at the repo root. The other comes from `.serena/cache/rust/document_symbols.pkl`. The "exactly one line" gate fails on a correct change. The implementer then deletes local caches to pass it, or waives the gate. Routed to Feasibility, `[UNSTATED_ASSUMPTION]` (BLOCKER).
2. The new `AGENTS.md` bullet tells every agent session that docker-db ignores the variable "on every CI matrix version". The plan's evidence comes from 2025.1.16, but CI and `db_tag()` run 2025.1.14. The PR reviewer also looks for an `IT (2025.1.16)` job that never runs. Routed to Feasibility, `[UNSTATED_ASSUMPTION]` (BLOCKER).
3. A later docker-db release changes its default `MemSize`. The plan writes "fixed 2 GiB" into four files: two comments in `ci.yml`, `benches/README.md`, and `AGENTS.md`. The copies go stale together, the same way the 4 GiB comments did. Routed to Design Depth, `[INFORMATION_LEAKAGE]` (ADVISORY).

## Intent Fidelity

The plan covers every place the issue and the interview name: `crates/it/src/lib.rs`, `ci.yml`, `ci-it-local.sh` with its `DB_MEM` knob, `benches/README.md`, and `AGENTS.md`. It has no spec delta and no version bump, as the user chose. `PERF.md`, `perf-spike/`, and `COSLWD_ENABLED=1` stay untouched, as the user chose. No intent BLOCKER.

#### [SCOPE_CREEP] ADVISORY
- Location: plan.md § Implementation Tasks, task 1.6; decision-log.md § [1] Consequences
- Issue: Interview Q2 says "`AGENTS.md` gets the harness fact". Task 1.6 adds a second claim to that bullet: "a custom EXAConf `MemSize` is the only known way to change DB RAM, and that this path is untested". `AGENTS.md` is loaded into every agent session. This claim is unverified guidance, and "only known way" has no evidence in the plan. With it, task 1.6 needs four facts in "one or two sentences".
- Fix: In plan.md task 1.6, drop the clause about the untested EXAConf `MemSize` route, so the bullet states only that docker-db ignores `EXA_DB_MEM_SIZE`, that the DB runs with the 2 GiB `MemSize` the image writes, and that contributors must not pass the variable. In decision-log.md [1] Consequences, replace "which `AGENTS.md` names" with "which issue #127 describes".

## Feasibility

#### [UNSTATED_ASSUMPTION] BLOCKER
- Location: plan.md § Context, bullet 2 (line 10); plan.md § Verification › Manual Testing, row "CI workflow" (line 87); decision-log.md § [1] Rationale (line 20); plan.md task 1.6
- Issue: plan.md says "Planning verified this on all three CI matrix images (8.29.13, 2025.1.16, 2026.1.1)". CI does not run 2025.1.16. `.github/workflows/ci.yml:371` sets `exasol_version: "2025.1.14"`, and `crates/it/src/lib.rs:47` maps `"2025-1"` to `"2025.1.14"`. The Manual Testing row expects `IT (2025.1.16)` to be green, but the job is named `IT (${{ matrix.exasol_version }})`, so CI shows `IT (2025.1.14)`. Task 1.6 then writes "on every CI matrix version" into `AGENTS.md`, resting on a probe of an image CI does not use. The conclusion still holds. The reviewer checked `exasol/docker-db:2025.1.14`: `libconfd/EXAConf.py:1717` sets `mem_size` to 2 GiB per node, and no file under `/opt/exasol` contains `EXA_DB_MEM_SIZE`. The evidence text and the expected CI job name are wrong.
- Fix: In plan.md § Context bullet 2 and in decision-log.md [1] Rationale, replace 2025.1.16 with 2025.1.14 as the CI matrix image. Either re-run the planning probe on 2025.1.14 (`docker run -e EXA_DB_MEM_SIZE='3 GiB' exasol/docker-db:2025.1.14`, then `grep MemSize /exa/etc/EXAConf`), or cite the static check: `libconfd/EXAConf.py:1717` sets 2 GiB per node, and no file under `/opt/exasol` reads `EXA_DB_MEM_SIZE`. You may keep 2025.1.16 as an extra checked image. In plan.md § Verification › Manual Testing, row "CI workflow", change the expected jobs to `IT (8.29.13)`, `IT (2025.1.14)`, `IT (2026.1.1)`.
- Escalation: MECHANICAL. The matrix version is in `ci.yml` and `db_tag()`, and the reviewer verified the 2025.1.14 default. No requester judgment is needed.

#### [UNSTATED_ASSUMPTION] BLOCKER
- Location: plan.md § Implementation Tasks, task 2.1 (line 48)
- Issue: Task 2.1 expects its `grep -rnE "EXA_DB_MEM_SIZE|DB_MEM" ...` to print "exactly one line, the new `AGENTS.md` bullet". It assumes the tree holds no untracked binary that contains the string. Today the same command prints `binary file matches` for two gitignored files. One is `/home/crusty/code/slc-rs/it-runner`, a 360 MB copy of the IT test binary from an earlier `ci-it-local.sh` run, built 2026-09-29 from code that still contains `EXA_DB_MEM_SIZE`. The other is `/home/crusty/code/slc-rs/.serena/cache/rust/document_symbols.pkl`. Task 2.1 runs before task 2.6, the only step that rebuilds `it-runner`. Serena's cache can keep the old symbol text after the edit. So the gate fails on a correct implementation.
- Fix: In plan.md task 2.1, search tracked files only: `git -C /home/crusty/code/slc-rs grep -nE "EXA_DB_MEM_SIZE|DB_MEM" -- . ':!PERF.md' ':!perf-spike' ':!specs/_recorded' ':!specs/_plans'`. Keep the expected result: exactly one line, the new `AGENTS.md` bullet. As an alternative, add `-I` (skip binary files) to the existing `grep` and state that gitignored binaries such as `it-runner` are out of scope.
- Escalation: MECHANICAL. The reviewer reproduced both extra matches by running the plan's own command. Fixing it is a command edit.

#### [HIDDEN_DEPENDENCY] ADVISORY
- Location: plan.md § Verification › Checklist, row "Test" (line 94)
- Issue: The row runs `cargo test -p it --features integration` with no environment and expects "0 failures". The harness fails fast without `SLC_TARBALL` (`crates/it/src/lib.rs:206-211`). It also needs the fixtures that `scripts/build-test-udfs.sh` builds into `target/release/`. Run as written, the checklist command fails. Task 2.5 has the full recipe, but the checklist does not point to it.
- Fix: In plan.md § Verification › Checklist, row "Test", replace the integration command with `SLC_TARBALL=<dir>/lc-rs.tar.gz EXASOL_VERSION=2026.1.1 cargo test -p it --features integration (after task 2.5's tarball and fixture builds)`.

## Requirement Quality

no objection — axis checked: the plan has no spec delta, as the user chose (interview Q2). `speq plan validate fix-db-mem-size-noop` passes with "No delta specs found in plan". `AGENTS.md` § Specs and issues places test-harness and CI mechanics outside `specs/`. No delta exists that could be ambiguous, incomplete, conflicting, or leaky.

## Task Breakdown

#### [TRACEABILITY_GAP] ADVISORY
- Location: decision-log.md § [2] Decision (line 28); plan.md tasks 1.1 and 1.4; plan.md § Summary
- Issue: Decision [2] says "The comments in `crates/it/src/lib.rs` and `ci.yml`, the header of `ci-it-local.sh`, and `benches/README.md` state that the DB runs with docker-db's fixed 2 GiB RAM". Two tasks do not write that text. Task 1.1 replaces the `lib.rs` comment with a `shm`-only line, `// shm holds the UDF sandbox; 2 GiB matches ci.yml and ci-it-local.sh.`. Task 1.4 only deletes DB RAM wording from the `ci-it-local.sh` header and adds nothing. The implementer gets two different instructions for the same comment. Also, the `2 GiB` in the new `lib.rs` line refers to shm, right where a 2 GiB DB RAM fact now applies.
- Fix: In decision-log.md [2] Decision, say that only the `ci.yml` comments and `benches/README.md` state the 2 GiB DB RAM, and that `lib.rs` and the `ci-it-local.sh` header drop the DB RAM wording without a replacement. In plan.md § Summary, change "Comments, `benches/README.md`, and `AGENTS.md` then state the real 2 GiB default" to name only the `ci.yml` comments, `benches/README.md`, and `AGENTS.md`. In task 1.1's example comment, write `shm` next to the size, for example `// shm holds the UDF sandbox; its 2 GiB size matches ci.yml and ci-it-local.sh.`

The single Parallelization group is coherent. All tasks share one fact and touch the same five files, and the tasks are small enough to verify one by one. No `[CLUSTER_INCOHERENCE]` or `[TASK_GRANULARITY]` finding.

## Design Depth

#### [INFORMATION_LEAKAGE] ADVISORY
- Location: plan.md tasks 1.2, 1.3, 1.5, 1.6; decision-log.md § [2] Alternatives
- Issue: Decision [2] rejects repeating one external-system fact because "four copies of one external-system fact drift apart". The plan still writes the number "2 GiB" for DB RAM into four places: the `integration` job comment (task 1.2), the "Start Exasol" comment (task 1.3), `benches/README.md` (task 1.5), and `AGENTS.md` (task 1.6). The number comes from docker-db's `EXAConf.py` (2 GiB per node). Nothing in the repo checks it. If a matrix bump changes the default, all four copies go stale together. The `benches/README.md` label needs the number, because it records a measurement condition. The second `ci.yml` copy does not.
- Fix: In plan.md task 1.2, have the `integration` job comment say the DB runs with docker-db's default RAM, and point to the "Start Exasol" step below for the size, as the old comment did. Leave the "2 GiB" number to task 1.3 within `ci.yml`.

No new module, interface, or boundary is introduced, so the Quick Diagnostic table does not apply. `[ADR_OVERPROMOTION]`: checked. All five decision-log entries say `Promotes to ADR: no`. `[ARCHITECTURE_DRIFT]`: checked. No architecture delta exists, `specs/architecture.md` describes `it` and the `exasol/docker-db` dependency without memory details, and `plan.md` adds no component, boundary, interface, or dependency. `[ADR_CONFLICT]`: checked. `speq decision-log show` contains no ADR on DB memory, docker-db sizing, or harness environment. The single-tarball ADR that names `ci-it-local.sh` and `SLC_TARBALL` is unaffected.

## Prose Quality

#### [PROSE_BLOAT] ADVISORY
- Location: decision-log.md § [5] Alternatives (line 50); plan.md § Context, bullet 9 (line 17)
- Issue: Decision [5] ends with "The search index still lists stale `integration/db-roundtrip` scenarios, but no such directory exists on disk." That sentence does not bear on the decision to add no spec delta. Context bullet 9 ("Every recorded IT run and benchmark ran on a 2 GiB database...") repeats decision-log [1] Consequences and [3] Rationale. Neither file uses em dashes.
- Fix: Delete the stale-search-index sentence from decision-log.md [5] Alternatives, and open a separate issue if the index needs a rebuild. Delete plan.md § Context bullet 9, or cut it to "CI is green on the 2 GiB default today."
