# Tasks: fix-db-mem-size-noop

## PR Lifecycle
- [x] resolved
- [x] implemented
- [x] version-bumped
- [ ] tested-green
- [ ] recorded
- [ ] pr-ready

## Phase 2: Implementation (Group A)
- [x] 1.1 `crates/it/src/lib.rs` `Harness::start`: remove `db_mem` and `EXA_DB_MEM_SIZE` env var, one-line `shm` comment
- [x] 1.2 `.github/workflows/ci.yml` integration job comment: state fixed 2 GiB DB RAM
- [x] 1.3 `.github/workflows/ci.yml` "Start Exasol" step: drop `-e EXA_DB_MEM_SIZE`, rewrite comment
- [x] 1.4 `scripts/ci-it-local.sh`: remove `DB_MEM` knob, array, log field, usage-example wording
- [x] 1.5 `benches/README.md`: line 51 and profile table header (2 GiB)
- [x] 1.6 `AGENTS.md` Testing: add bullet on docker-db ignoring `EXA_DB_MEM_SIZE`

## Phase 3: Verification
- [x] 2.1 tracked-file grep for `EXA_DB_MEM_SIZE|DB_MEM` prints exactly the AGENTS.md line
- [x] 2.2 grep `4 GiB|auto-siz` prints nothing
- [x] 2.3 `bash -n` script and YAML parse
- [x] 2.4 fmt + clippy
- [x] 2.5 Docker-mode integration run
- [x] 2.6 External-mode `ci-it-local.sh` run

## Phase 4: Review Fixes
- [x] 4.1 `.github/workflows/ci.yml`: replace `Standard ubuntu-latest (2 vCPU / 7 GB)` with `Standard ubuntu-latest (4 vCPU / 16 GB for a public repo)` in the integration job comment and `fits the 7 GB runner` with `fits the 16 GB runner` in the "Start Exasol" step comment
