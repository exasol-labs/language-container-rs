# Code Review Findings: fix-db-mem-size-noop

## Summary
- Files reviewed: 5
- Total findings: 1 (standard: 1, expert: 0)

The five edits match plan tasks 1.1 to 1.6. The reviewer checked them with these commands, and each passed:

- Task 2.1 grep: `git grep -nE "EXA_DB_MEM_SIZE|DB_MEM" -- . ':!PERF.md' ':!perf-spike' ':!specs/_recorded' ':!specs/_plans'` prints one line, `AGENTS.md:15`.
- Task 2.2 grep: `grep -rnE "4 GiB|auto-siz" crates scripts benches .github AGENTS.md` prints nothing.
- `bash -n scripts/ci-it-local.sh` exits 0.
- `cargo clippy -p it --all-targets --features integration -- -D warnings` finishes clean.
- `cargo fmt --all -- --check` exits 0.

`shellcheck` is not installed on this host, so the reviewer did not run it.

## Standard fixes

### .github/workflows/ci.yml

#### [OUTDATED_COMMENT] Runner size in the integration job comments does not match GitHub's ubuntu-latest runner
- Location: line 355 (`integration` job comment) and line 472 ("Start Exasol" step comment)
- Issue: The new step comment says "docker-db's fixed 2 GiB DB RAM fits the 7 GB runner." The job comment it pairs with says "Standard ubuntu-latest (2 vCPU / 7 GB)". The repository `exasol-labs/language-container-rs` is public (`gh repo view --json visibility` returns `PUBLIC`). GitHub's runner reference (docs.github.com/en/actions/reference/runners/github-hosted-runners) lists ubuntu-latest for public repositories as 4 CPUs and 16 GB RAM. Private repositories get 2 CPUs and 8 GB, so neither row says 7 GB. The conclusion that 2 GiB fits still holds. The figure the step comment gives as its reason is wrong.
- Fix: In `.github/workflows/ci.yml`, replace `Standard ubuntu-latest (2 vCPU / 7 GB)` on line 355 with `Standard ubuntu-latest (4 vCPU / 16 GB for a public repo)`. Replace `docker-db's fixed 2 GiB DB RAM fits the 7 GB runner.` on line 472 with `docker-db's fixed 2 GiB DB RAM fits the 16 GB runner.` Leave every other line of both comments unchanged. Then run `python3 -c "import yaml; yaml.safe_load(open('.github/workflows/ci.yml'))"` and confirm it exits 0.

## Expert fixes
[none]
