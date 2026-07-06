# Verification Report: add-emit-transfer-spikes

**Generated:** 2026-07-06

## Verdict

| Result | Details |
|--------|---------|
| **PASS** | Spike-and-measure resolved issue #29 with real data: string-block formatting fast-path promoted (+28–46% emit throughput), Arrow C Data Interface and raw per-column buffer spikes measured, rejected, and deleted. Ingest side symmetrically productionized. All checks green. |

| Check | Status |
|-------|--------|
| Build | ✓ |
| Tests | ✓ |
| Lint | ✓ |
| Format | ✓ |
| Scenario Coverage | ✓ |
| Manual Tests | ✓ |

## Test Evidence

### Test Results

| Type | Run | Passed | Ignored/Failed |
|------|-----|--------|---------|
| Unit (`cargo test`, full workspace) | 50 suites | 200 | 4 ignored, 0 failed |
| Integration (`cargo test -p it --features integration`, live Exasol 2026.1.0 Docker) | 3 suites | 6 | 0 failed |

### Manual Tests

| Test | Result |
|------|--------|
| `docker build -f Dockerfile.alpine --target artifact` (SLC builds after Dockerfile ARG cleanup) | ✓ |
| `benches/emit-bench` extended shape matrix (mixed + wide) against live Exasol, baseline + 3 spikes | ✓ — see Notes for the measured numbers |
| `cargo build --release -p exaudfclient` | ✓ |
| `cargo build --release` (full workspace) | ✓ |

## Tool Evidence

### Linter

```
cargo clippy --all-targets --all-features -- -D warnings
No issues found
```

### Formatter

```
cargo fmt --check
(clean, no output)
```

## Scenario Coverage

| Domain | Feature | Scenario | Test Location | Test Name | Passes |
|--------|---------|----------|---------------|-----------|--------|
| runtime | dispatch-run-loop | A promoted emit fast-path encoder stays byte-identical to the row path | `crates/exa-udf-runtime/src/rowset.rs` (`fast_string_block_tests`) | `fast_path_to_proto_byte_identical_to_row_path`, `fast_decimal_matches_display_for_all_cases`, `fast_date_matches_chrono_format`, `fast_timestamp_matches_chrono_format`, `value_to_block_string_matches_slow_path_for_numeric_date_timestamp` | Pass |
| runtime | dispatch-run-loop | A promoted emit fast-path encoder stays byte-identical to the row path | `crates/it/tests/db_roundtrip.rs` | `numeric_date_timestamp_emit_roundtrips` | Pass |
| runtime | dispatch-run-loop | A promoted ingest fast-path decoder round-trips byte-identically | `crates/exa-udf-runtime/src/rowset.rs` (`fast_string_block_ingest_tests`) | `fast_parse_date_matches_chrono_parse_for_valid_dates`, `fast_parse_timestamp_matches_chrono_parse_for_valid_timestamps`, `decode_string_block_preserves_leniency_when_fast_path_defers`, `malformed_date_and_timestamp_strings_decode_to_null` | Pass |
| runtime | dispatch-run-loop | A promoted ingest fast-path decoder round-trips byte-identically | `crates/it/tests/db_roundtrip.rs` | `numeric_date_timestamp_ingest_roundtrips` | Pass |

## Notes

**Measured spike results** (live Exasol 2026.1.0 Docker, `wide` shape — `id BIGINT, amount
DECIMAL(18,2), event_date DATE, event_ts TIMESTAMP, label VARCHAR(100)`, N=1,000,000,
reduced-config single run, see `decision-log.md` Design Decision [8] for full methodology):

| Configuration | Mode | rows/s | MB/s | vs. baseline |
|---|---|---|---|---|
| Baseline (Arrow IPC, pre-plan) | row | 655,801 | 69.5 | — |
| Baseline | batch | 656,486 | 69.6 | — |
| **Promoted**: string-block fast-path | row | 959,326 | 101.7 | **+46%** |
| Promoted | batch | 839,368 | 89.0 | **+28%** |
| Rejected: Arrow C Data Interface | batch | 558,905 | 59.2 | **−15%** |
| Rejected: raw per-column buffers | batch | 609,038 | 64.6 | **−7%** |

This reinforces (does not overturn) the prior fix-abi-feature-safety ADR (2026-06-25): the
`.so`↔host transport was re-measured on the NUMERIC/DATE/TIMESTAMP types the original
"2–9%" benchmark never covered, and both FFI/raw-buffer transport changes still
underperform the status quo. The dominant, now-fixed cost was per-cell string-block
formatting (`chrono` generic `.format()`/`parse_from_str`, `Decimal`'s generic `Display`),
not the transport mechanism.

**Known limitation, out of scope for this plan:** `benches/emit-bench`'s ingest measurement
(task 1.2) has a pre-existing, reproducible bug — `sink_<shape>(emit_<shape>_<mode>(n,1))`
UDF-to-UDF chaining truncates at the first ~4,000,000-byte `MT_EMIT` flush from the
upstream UDF instead of reading to end of input — blocking a live throughput number for
the ingest-side fast-path (correctness is fully proven via unit tests + a live DB
round-trip integration test; only the *speedup magnitude* lacks a live number). Recommend
filing a separate GitHub issue.

**GitHub issue #29** was commented on with this resolution and awaits manual closing (the
`ghbrk` broker policy for this repo forbids `issue_close`, by design — not something this
session could or should route around).

**ABI**: `EXA_UDF_ABI_VERSION` was bumped 6→7 mid-plan (to cover Spike B's and Spike C's
new trait methods) and reverted back to 6 once both were deleted — the shipped state has
no net ABI change from before this plan.
