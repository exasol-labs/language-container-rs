# Verification Report: add-scalar-connect-back

## Verdict: PASS

Connect-back from a Rust `SCALAR` UDF is verified working with no runtime code change. All
plan tasks complete; all automated checks green; the new integration scenario passes live.

## Evidence

### Automated checks
| Check | Command | Result |
|-------|---------|--------|
| Build | `cargo build --release` | exit 0 |
| Test | `cargo test` | all pass* |
| Lint | `cargo clippy --all-targets --all-features -- -D warnings` | 0 warnings |
| Format | `cargo fmt --check` | clean |

\* Two `exa-udf-runtime` dispatch tests initially failed on stale `target/debug/lib*.so`
fixtures (ABI 3 vs host ABI 4) — a pre-existing artifact-staleness issue unrelated to this
change. Rebuilding the debug fixtures (`cargo build -p scalar-double -p annotated-fixture`)
resolved both; full suite green.

### Scenario coverage
| Scenario | Test | Result |
|----------|------|--------|
| Connect-back SCALAR UDF queries the database and returns the result | `connect_back_scalar_queries_and_returns` (db_roundtrip.rs) | PASS — returned "42" |
| connect-back-scalar sample builds + loads | `cargo build --release -p connect-back-scalar` | PASS — `libconnect_back_scalar.so` |
| Connect-back identical in scalar and set dispatch | scalar + set scenarios both green | PASS |

### Live integration run (this session)
Full `db_roundtrip_all_scenarios` against `exasol/docker-db:2026.1.0` (external mode):
`test result: ok. 1 passed` — all 15 scenarios green, including `[it] scenario connect_back_scalar ok`.
No SIGABRT.

## Notes
- No runtime code changed. Deliverables: sample UDF crate, IT scenario, CI build-list wiring,
  CLAUDE.md rule relaxation, docs/writing-a-udf.md correction.
- Root cause of the stale "never SCALAR" rule: the historical SIGABRT was the loopback
  connect-back address (fixed in ADR-029), not the UDF type. Recorded as ADR-040.
