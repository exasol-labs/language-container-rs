# Plan: fix-db-mem-size-noop

## Summary

Resolve GitHub issue #127 by removing the `EXA_DB_MEM_SIZE` setting from the integration harness, CI, and the local CI replay script, because `exasol/docker-db` ignores it and always runs the database with 2 GiB of RAM. Comments, `benches/README.md`, and `AGENTS.md` then state the real 2 GiB default.

## Context

- `exasol/docker-db` reads its database RAM from `MemSize` in `/exa/etc/EXAConf`. The image writes `MemSize = 2 GiB` at init, whatever `EXA_DB_MEM_SIZE` holds. The boot log prints the variable among the environment, but no init step applies it.
- The CI matrix runs `exasol/docker-db` 8.29.13, 2025.1.14, and 2026.1.1 (`ci.yml` matrix, `crates/it` `db_tag()`). The evidence covers all three:
  - Planning started 8.29.13 and 2026.1.1 with `-e EXA_DB_MEM_SIZE='3 GiB'`. Each container had the variable in its environment and `MemSize = 2 GiB` in EXAConf. On 2026.1.1, after full boot, `SELECT EVENT_TYPE, DB_RAM_SIZE FROM EXA_SYSTEM_EVENTS` returned `STARTUP, 2.0`.
  - Plan review checked the 2025.1.14 image statically. Its `libconfd/EXAConf.py:1717` sets `mem_size` to 2 GiB per node, and no file under `/opt/exasol` contains `EXA_DB_MEM_SIZE`.
  - Planning also started 2025.1.16, which is not in the matrix, with the same variable. Its EXAConf also held `MemSize = 2 GiB`.
- Four places pass or document the variable as if it worked:
  - `crates/it/src/lib.rs` `Harness::start` passes `EXA_DB_MEM_SIZE` (default `4 GiB`). Its comment says this replaces docker-db auto-sizing, but docker-db does not auto-size.
  - `.github/workflows/ci.yml` passes `-e EXA_DB_MEM_SIZE='4 GiB'`. Two comments say the DB "is pinned to 4 GiB" and that "pinning DB RAM is what bounds the footprint".
  - `scripts/ci-it-local.sh` has a `DB_MEM` knob that sets the variable. Its header says leaving it unset lets docker-db auto-size.
  - `benches/README.md` says Docker mode takes `EXA_DB_MEM_SIZE` with a 4 GiB default and labels the profile timings "4 GiB".
- `udf-bench` boots the database through `it::Harness::start`, so the `crates/it` edit also covers the benchmark Docker mode.
- Every recorded IT run and benchmark ran on a 2 GiB database, because the variable had no effect. CI is green on that configuration today.
- `PERF.md` and the `perf-spike/` scripts and logs also quote 4 GiB and `DB_MEM`. Those labels are inaccurate, but the user left both out of scope, together with `COSLWD_ENABLED=1`.
- `AGENTS.md` assigns test-harness and CI mechanics to itself, not to `specs/`. No spec scenario or accepted ADR covers database memory sizing.
- `specs/architecture.md` names `crates/it` and `exasol/docker-db` without memory details, so the plan has no architecture delta.

## Features

| Feature | Status | Spec |
|---------|--------|------|
| None | n/a | No spec delta: the change touches only the test harness, CI, a script, and docs (decision-log [5]) |

## Impact

None for users of the SDK, runtime, CLI, or container image. No version bump: the change is tooling and docs only.

Contributors see no behavior change either: the database already ran with 2 GiB. Setting `EXA_DB_MEM_SIZE` before `cargo test -p it` or `udf-bench`, or `DB_MEM` before `scripts/ci-it-local.sh`, is now silently ignored, which matches its earlier effect. The `PERF.md` command line that sets `DB_MEM='4 GiB'` keeps working for the same reason.

## Dependencies

- Task 2.5 needs Docker and `cargo-about` (used by `dist/generate-licenses.sh`). Task 2.6 also needs `jq` and `exapump`, which `scripts/ci-it-local.sh` already requires. All four are installed on the planning host.

## Implementation Tasks

1. Remove the setting and correct the text (one knowledge cluster: the docker-db memory fact)
   - [ ] 1.1 `crates/it/src/lib.rs` `Harness::start`: delete the `db_mem` binding and the `.with_env_var("EXA_DB_MEM_SIZE", db_mem)` call. Replace the three-line comment with a one-line `shm` rationale, for example `// shm holds the UDF sandbox; 2 GiB matches ci.yml and ci-it-local.sh.` The comment does not name `EXA_DB_MEM_SIZE` (decision-log [2]).
   - [ ] 1.2 `.github/workflows/ci.yml` `integration` job comment (lines 356-361): replace "and the DB is pinned to 4 GiB (see EXA_DB_MEM_SIZE below)" with a statement that docker-db runs the DB with its fixed 2 GiB RAM. Keep the AppArmor sentences unchanged.
   - [ ] 1.3 `.github/workflows/ci.yml` "Start Exasol" step (lines 469-483): delete the `-e EXA_DB_MEM_SIZE='4 GiB' \` line. Replace the four-line `EXA_DB_MEM_SIZE` comment with one or two lines: no `--memory` cgroup cap, like exarrow-rs on this runner class, and docker-db's fixed 2 GiB DB RAM fits the 7 GB runner. Keep the `--shm-size` comment, the `COSLWD_ENABLED mirrors exarrow-rs.` line, and `-e COSLWD_ENABLED=1` unchanged.
   - [ ] 1.4 `scripts/ci-it-local.sh`: delete the `DB_MEM` knob row in the header (line 23), the `DB_MEM=` default (line 41), the `DB_MEM_ARG` array and its `[ -n "$DB_MEM" ]` line (lines 95-96), and the `"${DB_MEM_ARG[@]}" \` argument (line 101). Drop `DB_MEM='${DB_MEM:-<auto>}'` from the config log line (line 56). In the usage examples (lines 11 and 14-15), drop "DB RAM auto-sized", "pin DB RAM", and `DB_MEM='4 GiB'`, so they read `reproduce the bug (broken CI config: 6g cap)` and `validate the fix (generous ceiling)` with `MEM=12g SHM=2g scripts/ci-it-local.sh`. Leave the `--memory` cgroup narrative (lines 5-8, 17, 29-32) unchanged (decision-log [4]).
   - [ ] 1.5 `benches/README.md` line 51: replace "(`EXASOL_VERSION`, `EXA_DB_MEM_SIZE`, default 4 GiB)" with text naming `EXASOL_VERSION` and stating that the DB runs with the image's fixed 2 GiB RAM. In the profile table header (line 55), change "docker-db 2026.1.1, 4 GiB, incl. Docker start" to "docker-db 2026.1.1, 2 GiB, incl. Docker start". Keep the timing values (decision-log [3]).
   - [ ] 1.6 `AGENTS.md` `## Testing`, "Project specifics" list: add one bullet after the `cargo test -p it --features integration` bullet. It states that `exasol/docker-db` ignores `EXA_DB_MEM_SIZE` on every CI matrix version, so the DB always runs with the 2 GiB `MemSize` from the image's default EXAConf. The "every CI matrix version" claim rests on Context bullet 2: the probes of 8.29.13 and 2026.1.1, and the static check of 2025.1.14 (`libconfd/EXAConf.py:1717`, no reader of the variable under `/opt/exasol`). It says not to pass the variable. It says a custom EXAConf `MemSize` is the only known way to change DB RAM, and that this path is untested. Keep it to one or two sentences per the `AGENTS.md` style.
2. Verify
   - [ ] 2.1 Grep over tracked files only: `git -C /home/crusty/code/slc-rs grep -nE "EXA_DB_MEM_SIZE|DB_MEM" -- . ':!PERF.md' ':!perf-spike' ':!specs/_recorded' ':!specs/_plans'` prints exactly one line, the new `AGENTS.md` bullet. `git grep` skips untracked and gitignored files, so stale local binaries such as `it-runner` and the `.serena/cache` symbol cache cannot match. Before the change, the same command prints 13 lines, all in the files that tasks 1.1-1.5 edit.
   - [ ] 2.2 Grep: `grep -rnE "4 GiB|auto-siz" crates scripts benches .github AGENTS.md` prints nothing.
   - [ ] 2.3 Static checks: `bash -n scripts/ci-it-local.sh` exits 0. `python3 -c "import yaml; yaml.safe_load(open('.github/workflows/ci.yml'))"` exits 0.
   - [ ] 2.4 Lint gates: `cargo fmt --all -- --check` and `cargo clippy --workspace --all-targets --all-features -- -D warnings` pass.
   - [ ] 2.5 Docker-mode integration run (exercises task 1.1). Build the SLC tarball the way CI does: `bash dist/generate-licenses.sh` (the Dockerfile copies its gitignored output), then `docker build --target artifact --output type=local,dest=<dir> .` with `<dir>` in the scratchpad. Build the fixtures with `bash scripts/build-test-udfs.sh`. Run `SLC_TARBALL=<dir>/lc-rs.tar.gz EXASOL_VERSION=2026.1.1 cargo test -p it --features integration` with `EXASOL_HOST` and `EXA_DB_MEM_SIZE` unset. Expect 0 failures.
   - [ ] 2.6 External-mode run (exercises task 1.4), after 2.5 so it reuses the tarball: `SKIP_SLC_BUILD=1 SLC_TARBALL=<dir>/lc-rs.tar.gz MEM=12g scripts/ci-it-local.sh`. Expect the config log line without `DB_MEM` and `Done (rc=0)`. If ports 8563 or 2581 are taken, add `DB_PORT=18563 BFS_PORT=12581`.

## Parallelization

| Group | Tasks | Depends on | Knowledge |
|-------|-------|------------|-----------|
| A: docker-db memory default | 1.1-1.6, 2.1-2.6 | none | no spec delta; `crates/it/src/lib.rs` (`Harness::start`), `.github/workflows/ci.yml` (`integration` job), `scripts/ci-it-local.sh`, `benches/README.md`, `AGENTS.md` (`## Testing`), `crates/it/tests/db_roundtrip.rs` (verification only) |

- One group: every edit expresses the same fact, so splitting would hand several agents the same knowledge. No task needs `[expert]`.

## Dead Code Removal

| Type | Location | Reason |
|------|----------|--------|
| Variable and builder call | `crates/it/src/lib.rs` `Harness::start`: `db_mem`, `.with_env_var("EXA_DB_MEM_SIZE", db_mem)` | docker-db ignores the variable |
| CI flag | `.github/workflows/ci.yml` "Start Exasol": `-e EXA_DB_MEM_SIZE='4 GiB'` | docker-db ignores the variable |
| Script knob | `scripts/ci-it-local.sh`: `DB_MEM`, `DB_MEM_ARG`, header row, log field | Its only effect was passing the ignored variable |

## Verification

### Scenario Coverage

No spec scenarios: this plan has no spec delta (decision-log [5]). Tasks 2.5 and 2.6 run the existing `db_roundtrip_all_scenarios` integration test in `crates/it/tests/db_roundtrip.rs` to show the suite stays green on the 2 GiB default.

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| (none: existing suite as regression check) | Integration | `crates/it/tests/db_roundtrip.rs` | `db_roundtrip_all_scenarios` |

### Manual Testing

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| IT harness, Docker mode | `SLC_TARBALL=<dir>/lc-rs.tar.gz EXASOL_VERSION=2026.1.1 cargo test -p it --features integration` | `db_roundtrip_all_scenarios ... ok`, and every `test result:` line reports `0 failed` |
| Local CI replay script | `SKIP_SLC_BUILD=1 SLC_TARBALL=<dir>/lc-rs.tar.gz MEM=12g scripts/ci-it-local.sh` | Config line `MEM=12g MEMSWAP=12g SHM=2g IMAGE=exasol/docker-db:2026.1.1`, then `Done (rc=0)` |
| CI workflow | Push the branch and open the PR | `IT (8.29.13)`, `IT (2025.1.14)`, `IT (2026.1.1)` all green |

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Build | `cargo build --release` | Exit 0 |
| Test | `cargo test` and `cargo test -p it --features integration` | 0 failures |
| Lint | `cargo clippy --workspace --all-targets --all-features -- -D warnings` | 0 errors/warnings |
| Format | `cargo fmt --all -- --check` | No changes |
| Script syntax | `bash -n scripts/ci-it-local.sh` | Exit 0 |
| Workflow syntax | `python3 -c "import yaml; yaml.safe_load(open('.github/workflows/ci.yml'))"` | Exit 0 |
