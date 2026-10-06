# Verification Report: fix-db-mem-size-noop

## Verdict

| Result | Details |
|--------|---------|
| **PASS** | `EXA_DB_MEM_SIZE` and `DB_MEM` are removed from the harness, CI, and the local replay script. Comments and docs state the 2 GiB docker-db default. The integration suite passes on it in both Docker mode and external mode. |
| Code review | 1 finding, 1 fixed |

| Check | Status |
|-------|--------|
| Build | ✓ |
| Tests | ✓ |
| Lint | ✓ |
| Format | ✓ |
| Scenario Coverage | ✓ |
| Manual Tests | ✓ |

## Test Evidence

### Coverage

Not measured. The change touches no production code path: it removes one environment variable from the test harness and corrects text.

### Test Results

| Type | Run | Passed | Ignored |
|------|-----|--------|---------|
| Unit (`cargo test`) | 416 | 414 | 2 |
| Integration, Docker mode (`db_roundtrip_all_scenarios`, docker-db 2026.1.1) | 1 | 1 | 0 |
| Integration, external mode via `scripts/ci-it-local.sh` (docker-db 2026.1.1) | 1 | 1 | 0 |

`cargo test` reports 0 failed.

### Manual Tests

| Test | Result |
|------|--------|
| IT harness, Docker mode: `SLC_TARBALL=<dir>/lc-rs.tar.gz EXASOL_VERSION=2026.1.1 cargo test -p it --features integration`, `EXASOL_HOST` and `EXA_DB_MEM_SIZE` unset: `db_roundtrip_all_scenarios ... ok`, every `test result:` line `0 failed` (82.02 s) | ✓ |
| Local CI replay: `SKIP_SLC_BUILD=1 SLC_TARBALL=<dir>/lc-rs.tar.gz MEM=12g scripts/ci-it-local.sh`: config line `MEM=12g MEMSWAP=12g SHM=2g IMAGE=exasol/docker-db:2026.1.1` without `DB_MEM`, `db_roundtrip_all_scenarios` passed (43.73 s), `Done (rc=0)` | ✓ |
| CI workflow: `IT (8.29.13)`, `IT (2025.1.14)`, `IT (2026.1.1)` green on the pushed branch | Pending: runs on the PR |
| Tracked-file grep for `EXA_DB_MEM_SIZE\|DB_MEM` (excluding `PERF.md`, `perf-spike`, `specs/_recorded`, `specs/_plans`) prints exactly one line, `AGENTS.md:15` | ✓ |
| `grep -rnE "4 GiB\|auto-siz" crates scripts benches .github AGENTS.md` prints nothing | ✓ |
| `bash -n scripts/ci-it-local.sh` exits 0, and `yaml.safe_load` on `ci.yml` exits 0 | ✓ |

## Tool Evidence

### Linter

```
cargo clippy --workspace --all-targets --all-features -- -D warnings  → exit 0
```

### Formatter

```
cargo fmt --all -- --check  → exit 0
```

## Scenario Coverage

The plan has no spec delta. The existing `db_roundtrip_all_scenarios` test serves as the regression check.

| Domain | Feature | Scenario | Test Location | Test Name | Passes |
|--------|---------|----------|---------------|-----------|--------|
| (none) | (none) | existing suite as regression check | `crates/it/tests/db_roundtrip.rs` | `db_roundtrip_all_scenarios` | Pass |

## Notes

- Code review found that the `ci.yml` comments gave the `ubuntu-latest` size as 2 vCPU / 7 GB. The repository is public, so GitHub lists 4 vCPU / 16 GB. The fix updated both comments. The conclusion that the 2 GiB database fits the runner is unchanged.
- The Docker-mode and external-mode runs executed before the review fix. The fix changed only comment text in `ci.yml`, and the YAML parse and greps were rerun afterwards.
- The three-version CI matrix runs only on the PR. Local runs covered docker-db 2026.1.1. The 8.29.13 and 2025.1.14 evidence is in the plan's Context section.
- No version bump: the change is tooling and docs only.
