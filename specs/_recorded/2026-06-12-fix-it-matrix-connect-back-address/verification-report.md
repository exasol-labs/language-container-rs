# Verification Report: fix-it-matrix-connect-back-address

## Bottom line: PASS

The integration suite is **green locally in external mode** on `exasol/docker-db:2026.1.0` — the exact configuration CI runs. A single fix (connect-back address → container eth0 IP) makes all 12 scenarios pass, including `double_it`, confirming issue #2's "independent fatal scalar crash" was collateral damage from a poisoned session, not a separate bug.

```
test db_roundtrip_all_scenarios ... ok
  [it] python3_connect_back: CB_SELF address = 172.17.0.2:8563
  [it] scenario python3_connect_back ok        <- was the VM-crasher (localhost); now fine
  [it] scenario scalar_double ok               <- the "fatal" double_it; passes on a healthy session
  [it] scenario set_filter ok
  [it] scenario json_parse ok
  [it] scenario udf_error ok
  [it] scenario single_call_default_output_columns ok
  [it] scenario single_call_unimplemented ok
  [it] scenario connect_back_cluster_ip ok
  [it] scenario connect_back_dml ok
  [it] connect_back_query: CB_SELF address = 172.17.0.2:8563
  [it] scenario connect_back_query ok
  [it] scenario connect_back_writeback_same_schema ok
test result: ok. 1 passed; 0 failed; 0 ignored; finished in 15.20s
```

## Root cause (confirmed)

Failing CI run 27425472641 log shows `python3_connect_back` and `double_it` failing with the **same DB Session ID** — proof they hit one poisoned shared connection, not two independent bugs. Scenario 2 connected back to `localhost:8563` → `127.0.0.1` → CoreDB proxy → VM SIGABRT; the next scenario (`double_it`) then ran on the dead session. Fix: `connect_back_sql_address()` now always returns `<container-eth0-ip>:8563` (172.17.0.2:8563 here), and the Python3 diagnostic runs on an isolated throwaway connection.

## Automated checks
- `cargo +1.91 build` (workspace) — exit 0, compiles against exarrow-rs 0.12.7.
- `cargo +1.91 check -p it --features integration,db-2026-1` — exit 0.
- `cargo +1.91 fmt -p it -- --check` — clean.
- `cargo +1.91 clippy -p it --features integration,db-2026-1` — clean.

## Scenario coverage
All `integration/connect-back` and `integration/db-roundtrip` scenarios have a passing test in the run above. The new `db-roundtrip` scenario "Non-fatal Python3 connect-back diagnostic is session-isolated" is exercised: the diagnostic ran on its own connection and the shared connection stayed healthy for every asserted scenario.

## Build provenance (glibc trap avoided)
Test-UDF `.so` were built inside `rust:1.91-bookworm` (glibc 2.36, matching the SLC), NOT on the Debian-13 host (glibc ~2.41), so they `dlopen` correctly in the Exasol sandbox. SLC image rebuilt with exarrow-rs 0.12.7. `it-runner` recompiled from the fixed source.

## Matrix scope
Verified locally on **2026.1.0** (newest; the version most likely to differ). The fix is version-independent (connect-back address), and CI run 27425472641 showed all three versions (2025.1.11 / 2025.2.1 / 2026.1.0) failing identically for this one cause — so the green result generalizes. The full 3-version matrix will be confirmed by CI on push; the other two can also be run locally on request.

## Code review outcome
9 findings. In-scope fixes applied: (1) `connect_back_sql_address()` now uses the `DB_PORT` constant instead of a magic `8563` literal; (2) stale `exarrow-rs` local-path / `v0.12.5` / `[patch.crates-io]` references in `specs/mission.md` and `specs/design.md` updated to crates.io v0.12.7. Pre-existing, out-of-scope (noted, not fixed): `single_call_unimplemented_returns_undefined` is a no-op test; `container_inner_ip()` does not validate IPv4 format.

## Housekeeping
Local `exasol-db` container removed; the unrelated `strata-rs-exasol-1` container was left untouched.

## Next
Ready for `/speq:record fix-it-matrix-connect-back-address` (merges the connect-back + db-roundtrip spec deltas into `specs/` and appends ADR-029 to the decision log). Push to confirm the full CI matrix is green — pending your go-ahead.
