# Verification Report: fix-timestamp-timezone-handling

Date: 2026-06-19

## Bottom Line

**PASS — E2E gate met. Ready to record.**

Both fixes are implemented and proven end-to-end against a live
`exasol/docker-db:2026.1.0`:

- **Timezone** — `tzdata` is bundled AND the SLC tarball is built with GNU
  `tar --hard-dereference`, so the ~258 hardlinked IANA zones survive BucketFS
  extraction. `udf_now()` resolves the session's named zone (Berlin offset, not
  UTC). Regression test `udf_local_time_matches_session_tz` **passes**.
- **Precision** — the emit path carries full nanosecond precision (`%.9f`). The
  realistic round-trip ceiling through a UDF is **microseconds**, because the DB
  delivers every UDF *input* column at `FF6` (microsecond) precision for all
  script languages — not an SLC limitation. `timestamp_precision_matrix_roundtrips`
  **passes** with the corrected expectation (p≤6 lossless; p=9 → `.123456000`).

## Automated Checks

| Step | Command | Result |
|------|---------|--------|
| Build | `cargo build --release` | PASS |
| Format | `cargo fmt --check` | PASS |
| Lint | `cargo clippy --workspace --exclude it --all-targets -- -D warnings` | PASS |
| Unit/integration tests | `cargo test --workspace --exclude it` | PASS |
| Emit unit test | `cargo test -p exa-udf-runtime --lib timestamp` | PASS (`timestamp_emit_nanosecond_roundtrip`) |

## E2E (`db-roundtrip` against live `exasol/docker-db:2026.1.0`)

`db_roundtrip_all_scenarios`: **1 passed; 0 failed** — all scenarios green,
including the three new timestamp scenarios.

| Scenario | Result | Evidence |
|----------|--------|----------|
| `timestamp_arithmetic_roundtrips` | PASS | `ts_add_second(… 09:30:15.250000) = … 09:30:16.250000` — sub-second survives decode/emit |
| `udf_local_time_matches_session_tz` | PASS | UDF wall-clock carries the Berlin offset (`chrono_offset_secs=7199`), within skew tolerance of `CURRENT_TIMESTAMP`; would report UTC without the `tzdata`/hardlink fix |
| `timestamp_precision_matrix_roundtrips` | PASS | p∈{0,3,6} lossless; p=9 returns `.123456000` (the FF6 input cap), matching `CAST(CAST(lit AS TIMESTAMP(6)) AS TIMESTAMP(9))` |

## Root-Cause Findings

### Finding A — Timezone: hardlink-drop in SLC extraction (FIXED)

A diagnostic UDF proved: the DB injects `TZ=Europe/Berlin`; chrono opens
`/usr/share/zoneinfo/Europe/Berlin`; but that file was **absent in the sandbox**
(`berlin_file=false`) while the directory existed, so chrono fell back to UTC.
Alpine `tzdata` stores ~258 of 603 zones as **hardlinks** (Berlin →
Atlantic/Jan_Mayen); the packager used BusyBox tar (hardlink references) and
**BucketFS extraction does not recreate hardlinks**, so those zones vanished.
Fix: build the tarball with GNU `tar --hard-dereference` (0 hardlink entries
remain). Re-verified: `berlin_file=true`, `chrono_offset_secs=7199`, test passes.

### Finding B — Precision: the DB delivers UDF inputs at microsecond precision (FF6), for all languages

The original plan premise — that `%.9f` makes `TIMESTAMP(9)` round-trip
losslessly — was wrong. It verified only the *output* path. Verified now against
the DB engine source `../db/Engine/src/exscript/pluggable/swigcontainers_int.h`:

- **Input** (`SWIGTableData::getTimestamp`, :779-781) formats every UDF input
  column with `YYYY-MM-DD HH24:MI:SS.FF6` — **microseconds, hardcoded**, in the
  shared SWIGVM layer used by all pluggable languages.
- **Output** (`SWIGResultHandler::setTimestamp`, :1064-1082) parses `FF9` then
  `trunc_to_fractional_seconds_precision(value, prec)` to the declared column
  precision — so emitting `%.9f` is correct and lets UDF-*generated* sub-µs
  values reach a `TIMESTAMP(9)` output.
- **Internal storage** is nanosecond (`Timestamp::get_nanosecond`).

Confirmed empirically across three languages with a `TIMESTAMP '…123456789'`
literal:

| Language | Type holds ns? | Received |
|----------|----------------|----------|
| Rust (chrono) | yes | `.123456000` |
| Python (`datetime`) | no (µs) | `microsecond=123456` |
| Java (`java.sql.Timestamp.getNanos()`) | **yes** | `nanos=123456000` |
| DB internal (`TO_CHAR … FF9`) | — | `.123456789` |

The Java row is decisive: `getNanos()` returns `123456000`, not `123456789`,
even though the type holds 9 digits — so the truncation is on the DB→UDF input
wire, not in the receiving language. The nanosecond support seen in Java Virtual
Schemas (e.g. DB2 VS #38, Exasol 8.32+) comes from the adapter generating its
own `TO_CHAR(col,'FF9')` — a separate, adapter-controlled path with no UDF
equivalent.

**Conclusion:** the Rust SLC matches the engine's shared UDF-input contract
exactly; we are not doing anything wrong. `%.9f` is retained (decision-log [2]
correction) and the precision-matrix test now encodes the true microsecond input
cap. Temporary `tz-probe` diagnostic crate + harness probes removed.
