# Decision Log: fix-db-mem-size-noop

## Interview

**Q:** How should the DB memory setting be resolved: drop it, or really apply it?
**A:** Drop it (Recommended). Remove `EXA_DB_MEM_SIZE` from `crates/it`, `ci.yml`, and `ci-it-local.sh` (and the `DB_MEM` knob), and correct comments and docs to say the DB runs with the 2 GiB image default. CI is green on 2 GiB today.

**Q:** The issue touches only test-harness, CI, and docs. How should the plan treat specs?
**A:** No spec delta; plan only (Recommended). The plan covers tasks and decisions for the harness, CI, script, and doc edits. `AGENTS.md` gets the harness fact. No version bump (tooling only).

**Q:** Which stale references should the plan also correct beyond the three places named in the issue? (multi-select: `benches/README.md`, `PERF.md`, `perf-spike/` scripts and logs, `COSLWD_ENABLED=1`)
**A:** Only `benches/README.md` (line 51 says Docker mode takes `EXA_DB_MEM_SIZE` with a 4 GiB default, and the profile table header says "docker-db 2026.1.1, 4 GiB"). `PERF.md`, `perf-spike/`, and `COSLWD_ENABLED=1` stay untouched.

## Design Decisions

### [1] Drop the setting instead of pinning DB RAM through EXAConf

- **Decision:** Remove `EXA_DB_MEM_SIZE` from the harness, CI, and the local replay script. The database keeps the image's 2 GiB default.
- **Alternatives:** Pin DB RAM by supplying an EXAConf with a different `MemSize`, for example through `exadt init-sc --config`. Rejected by the user: the path is untested, adds a generated config file to three consumers, and buys nothing because every test and benchmark already passes on 2 GiB.
- **Rationale:** The variable has no effect on any of the three CI matrix images: 8.29.13, 2025.1.14, and 2026.1.1. Planning started 8.29.13 and 2026.1.1 containers with `-e EXA_DB_MEM_SIZE='3 GiB'`, and both wrote `MemSize = 2 GiB` to `/exa/etc/EXAConf`. The 2026.1.1 container reported `DB_RAM_SIZE = 2.0` in `EXA_SYSTEM_EVENTS` after boot. Plan review checked the 2025.1.14 image statically: its `libconfd/EXAConf.py:1717` sets `mem_size` to 2 GiB per node, and no file under `/opt/exasol` contains `EXA_DB_MEM_SIZE`. A planning probe of 2025.1.16, which is not in the matrix, also wrote `MemSize = 2 GiB`. Removing the variable therefore changes no runtime configuration, only the text that describes it.
- **Consequences:**
  - Recorded IT and benchmark results stay valid. They were measured on 2 GiB.
  - A future need for more DB RAM starts from the EXAConf route, which `AGENTS.md` names.
- **Promotes to ADR:** no

### [2] `AGENTS.md` alone names `EXA_DB_MEM_SIZE`; other comments state only the 2 GiB default

- **Decision:** The fact "docker-db ignores `EXA_DB_MEM_SIZE`" lives in one `AGENTS.md` Testing bullet. The comments in `crates/it/src/lib.rs` and `ci.yml`, the header of `ci-it-local.sh`, and `benches/README.md` state that the DB runs with docker-db's fixed 2 GiB RAM, without naming the variable.
- **Alternatives:** Repeat "docker-db ignores `EXA_DB_MEM_SIZE`" beside each removal site as a warning. Rejected: four copies of one external-system fact drift apart, and a comment about code that no longer exists narrates history, which the `AGENTS.md` code-style rules forbid.
- **Rationale:** `AGENTS.md` owns test-harness and CI mechanics and is loaded into every agent session, so it is the place a contributor or agent reads before adding the variable back. One owner also makes the task 2.1 grep exact: one expected hit.
- **Promotes to ADR:** no

### [3] Relabel the benchmark timing column to 2 GiB and keep the numbers

- **Decision:** Change the `benches/README.md` profile table header from "docker-db 2026.1.1, 4 GiB, incl. Docker start" to "docker-db 2026.1.1, 2 GiB, incl. Docker start". The 86 s and 255 s values stay.
- **Alternatives:** Drop the memory label from the header. Rejected: the label records a measurement condition, and 2 GiB is the condition that held. Re-measure the timings. Rejected: the database configuration under which they were taken is unchanged.
- **Rationale:** `udf-bench` boots the database through `it::Harness::start`, which passed the ignored variable. The timings were therefore measured on a 2 GiB database, and only the label was wrong.
- **Promotes to ADR:** no

### [4] Edit `ci-it-local.sh` only where it describes DB RAM; add no guard for a stale `DB_MEM`

- **Decision:** Remove the `DB_MEM` knob, its argument array, its log field, its header row, and the DB RAM wording in the two usage examples. Keep the `--memory` cgroup narrative ("VM crashed" reproduction, "defaults reproduce the broken pre-fix config", the swap note). Do not add a check that rejects a set `DB_MEM`.
- **Alternatives:** Rewrite the header around the AppArmor root cause that `ci.yml` documents. Rejected as scope creep: the user scoped this issue to the memory setting, and the cgroup narrative concerns `--memory`, not `EXA_DB_MEM_SIZE`. Fail fast when `DB_MEM` is set. Rejected: the knob never had an effect, so ignoring it preserves the existing behavior, and the `PERF.md` command line that sets it keeps working.
- **Rationale:** The `docker run` that the script issues is the same before and after the change in every effective setting (`--memory`, `--memory-swap`, `--shm-size`, `COSLWD_ENABLED`). The "reproduce the bug" example ran with a 2 GiB DB before, and the "validate the fix" example differed from it only in `MEM=12g`. The edited labels describe what actually ran.
- **Promotes to ADR:** no

### [5] No spec delta, no new test, no version bump

- **Decision:** The plan carries no spec delta and adds no test. Evidence is the existing integration suite run in Docker mode and through `ci-it-local.sh` on the 2 GiB default, plus the greps in tasks 2.1 and 2.2. `[workspace.package].version` stays unchanged.
- **Alternatives:** Add a unit test asserting that `Harness::start` does not set `EXA_DB_MEM_SIZE`. Rejected: it pins an implementation detail of a test harness and guards against nothing observable. Add a scenario to a spec. Rejected: `AGENTS.md` keeps test-harness mechanics out of `specs/`, and no recorded feature covers the harness. The search index still lists stale `integration/db-roundtrip` scenarios, but no such directory exists on disk.
- **Rationale:** The change removes a no-op from tooling and corrects text. `AGENTS.md` bumps the version only for changes downstream users can observe, and this change has none.
- **Consequences:**
  - `PERF.md` (lines 152, 157, 684) and `perf-spike/` keep their 4 GiB and `DB_MEM` references, by user choice. Those figures describe runs that in fact used a 2 GiB database.
  - `COSLWD_ENABLED=1` stays in `ci.yml` and `ci-it-local.sh` unchanged.
- **Promotes to ADR:** no

## Review Findings

### [plan-review] Evidence cited 2025.1.16 instead of the 2025.1.14 CI matrix image

- **Finding:** plan.md Context bullet 2, the Manual Testing row "CI workflow", decision-log [1] Rationale, and task 1.6 named 2025.1.16 as a CI matrix image. CI runs 2025.1.14 (`ci.yml` matrix entry, `db_tag()` for series `2025-1`). The "every CI matrix version" claim in the planned `AGENTS.md` bullet therefore rested on an image CI does not use. The expected job `IT (2025.1.16)` does not exist.
- **Direction change:** Context bullet 2 and decision-log [1] Rationale name 8.29.13, 2025.1.14, and 2026.1.1 as the matrix. They cite plan review's static check of 2025.1.14 (`libconfd/EXAConf.py:1717` sets 2 GiB per node; no file under `/opt/exasol` contains `EXA_DB_MEM_SIZE`), and keep 2025.1.16 as an extra, non-matrix probe. The Manual Testing row expects `IT (8.29.13)`, `IT (2025.1.14)`, `IT (2026.1.1)`. Task 1.6 points to Context bullet 2 as the evidence for "every CI matrix version". The 2025.1.14 evidence is plan review's static check, not a container probe.
- **Promotes to ADR:** no

### [plan-review] Task 2.1 grep matched gitignored binaries

- **Finding:** Task 2.1's recursive `grep` also matched the gitignored `it-runner` test binary at the repo root and `.serena/cache/rust/document_symbols.pkl`. Both keep the old `EXA_DB_MEM_SIZE` text after a correct edit, so the "exactly one line" gate would fail on a correct implementation.
- **Direction change:** Task 2.1 runs `git -C /home/crusty/code/slc-rs grep -nE "EXA_DB_MEM_SIZE|DB_MEM" -- . ':!PERF.md' ':!perf-spike' ':!specs/_recorded' ':!specs/_plans'`, which searches tracked files only. The expected result stays exactly one line, the new `AGENTS.md` bullet. Run against the current tree, the command prints 13 lines, all in files that tasks 1.1-1.5 edit.
- **Promotes to ADR:** no
