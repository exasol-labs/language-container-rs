# Plan: fix-timestamp-timezone-handling

## Summary

Close the timezone integration gap (INTEGRATION_POINTS_REPORT §2.2 / gap #1) by bundling `tzdata` in the shipped Alpine SLC runtime image so the DB-supplied IANA `TZ` resolves correctly instead of silently falling back to UTC, and make the wire emit path precision-aware so `TIMESTAMP(p)` round-trips losslessly. Both fixes are proven by new real-DB end-to-end scenarios in the `db-roundtrip` suite — including a timezone regression test that reports UTC on the broken image and the correct Berlin offset on the fixed one.

## Context

The Exasol engine sets `TZ` from the session timezone for **every** UDF (`swigengine.cc` → `NSEXEC_ENV_TZ` → `TZ`), commonly as an IANA name like `Europe/Berlin`. The shipped `Dockerfile.alpine` runtime stage installs only `libzmq` + `ca-certificates` and bundles no zoneinfo database, so `chrono::Local`/`time` cannot resolve a named zone and silently return UTC — wrong local-time results with no error. The Rust runtime does not read `TZ` itself; the fix is packaging (`apk add --no-cache tzdata`).

Separately, the emit path hardcodes 6 fractional digits (`crates/exa-udf-runtime/src/rowset.rs:289`, `TIMESTAMP_EMIT = "%Y-%m-%d %H:%M:%S%.6f"`). A UDF reading `TIMESTAMP(9)` and emitting it back is truncated to microseconds. The decode path already uses `%.f`, which parses 0..9 digits losslessly; only the emit side is lossy.

The Exasol engine truncates an emitted timestamp to the output column's declared precision on receipt — VERIFIED against `../db/Engine/src/exscript/pluggable/`: `zmqcontainer.cc:675` reads the SLC's emitted `table.data_string(...)` and calls `out.setTimestamp(col, ...)`; `SWIGResultHandler::setTimestamp` (`swigcontainers_int.h:1064-1082`) parses the string with format `YYYY-MM-DD HH24:MI:SS.FF9` (accepting 0-9 fractional digits) and then applies `trunc_to_fractional_seconds_precision(value, m_types[col].prec)`. Therefore the fix is simply to emit all 9 available nanosecond digits (`%.9f`) and let the engine truncate to the actual column precision — no client-side per-precision formatting, and no need to thread the output column metadata into the encoder.

- **Goals** — DB-supplied named timezones resolve correctly in UDFs; the emit path carries full nanosecond precision (`%.9f`) so `TIMESTAMP(p)` round-trips losslessly after the engine's receipt-side truncation; both properties are proven against a real database.
- **Non-Goals** — `TIMESTAMP WITH LOCAL TIME ZONE` semantics (deferred — larger type-model question); `Dockerfile.debian` parity (not the shipped image; noted only); locale/`nsswitch` gaps #3–#4 from the report.

## Design

### Decision

#### Tzdata (packaging)

Add `tzdata` to the `apk add` in the `Dockerfile.alpine` runtime stage. No Rust code change. The example `timestamp-now` UDF obtains local wall-clock time via a crate that consults `/usr/share/zoneinfo` for a named `TZ` (`chrono::Local`, or `time` with its tz feature). Whether a fully-static musl binary actually resolves a named zone from bundled `tzdata` is the unverified risk; the `udf_now()` e2e scenario is the real proof and the broken-vs-fixed regression assertion is the gate.

#### Full-precision timestamp emit

The engine truncates emitted timestamps to the output column's precision on receipt (VERIFIED — see Context: `swigcontainers_int.h:1064-1082`, `zmqcontainer.cc:675`). The fix is therefore a one-line format change in `value_to_block_string` (`rowset.rs:323`, uses `TIMESTAMP_EMIT` defined at `:289`): change the constant from `TIMESTAMP_EMIT = "%Y-%m-%d %H:%M:%S%.6f"` to `"%Y-%m-%d %H:%M:%S%.9f"`.

- `%.9f` is a valid chrono specifier that always emits exactly 9 fractional digits. `NaiveDateTime` carries nanosecond precision, so this emits the full available precision; the engine then truncates to the actual column precision.
- No need to thread the output `ColumnMeta` precision into the encoder, and no manual fractional formatting: the `%.0f`/`%.1f`/trailing-dot edge cases that the original plan had to handle simply do not arise, because there is no per-precision format selection. The encoder signature stays `value_to_block_string(v)`.
- The decode side stays as-is (`TIMESTAMP_PARSE = "%Y-%m-%d %H:%M:%S%.f"` already parses 0-9 digits losslessly).
- Plain (unqualified) `TIMESTAMP` in Exasol defaults to precision 3; emitting 9 digits preserves existing behavior for it (the engine truncates 9→3, exactly as it previously truncated 6→3).

#### Patterns

| Pattern | Where | Why |
|---------|-------|-----|
| Emit full precision, let engine truncate | `value_to_block_string` `%.9f` | Engine applies `trunc_to_fractional_seconds_precision(value, prec)` on receipt (verified); over-emitting is lossless, under-emitting is not |
| Regression-by-construction e2e | `udf_now()` UTC-vs-Berlin assert | Proves zoneinfo resolution on the real static-musl binary, unprovable in a host unit test |

### Consequences

| Decision | Alternatives Considered | Rationale |
|----------|------------------------|-----------|
| Emit `%.9f` unconditionally | Thread `ColumnMeta` precision and format per-column (the original plan); manual fractional formatting with a `match p` table | Verified that the engine truncates to the column precision on receipt, so emitting all 9 digits is both correct for every precision and far simpler — no metadata threading, no chrono `%.Nf` limitation, no `p=0` trailing-dot edge case |
| Bundle `tzdata` (packaging) | Read `TZ` and load zone data in Rust | Report confirms `chrono`/`time` read `TZ` implicitly; the only missing piece is the zoneinfo files |
| Plain `TIMESTAMP` only in e2e | Also cover `TIMESTAMP WITH LOCAL TIME ZONE` | TZ-typed values are a larger type-model change; deferred per interview Q2 |

## Features

| Feature | Status | Spec |
|---------|--------|------|
| container/slim-image | CHANGED | delta: `container/slim-image/spec.md` → `specs/container/slim-image/spec.md` |
| runtime/dispatch-run-loop | CHANGED | delta: `runtime/dispatch-run-loop/spec.md` → `specs/runtime/dispatch-run-loop/spec.md` |
| examples/test-udfs | CHANGED | delta: `examples/test-udfs/spec.md` → `specs/examples/test-udfs/spec.md` |
| integration/db-roundtrip | CHANGED | delta: `integration/db-roundtrip/spec.md` → `specs/integration/db-roundtrip/spec.md` |

## Implementation Tasks

1. Create feature branch `feat/fix-timestamp-timezone-handling` off `main`.
2. Add `tzdata` to the `apk add --no-cache` line in the `Dockerfile.alpine` runtime stage (alongside `libzmq` and `ca-certificates`).
3. Make the timestamp emit full-precision in `crates/exa-udf-runtime/src/rowset.rs`: change the `TIMESTAMP_EMIT` constant from `"%Y-%m-%d %H:%M:%S%.6f"` to `"%Y-%m-%d %H:%M:%S%.9f"` and update its doc comment to note the engine truncates to the column precision on receipt. `value_to_block_string` keeps its `(v)` signature; no `ColumnMeta` threading.
4. Add a `#[cfg(test)]` unit test in `rowset.rs` proving a `Value::Timestamp` carrying sub-microsecond (nanosecond) precision now emits 9 fractional digits (not truncated at 6) and round-trips losslessly through `to_proto`→`from_proto`. Confirm the existing `bridge_typed_getters_return_typed_options` round-trip still passes (whole-second and microsecond values are unaffected).
5. Scaffold `test-udfs/timestamp-add-second` (reads `Value::Timestamp`, emits +1s; passes NULL through) with `#[cfg(test)]` `TestCtx` tests; cdylib for musl, exports `__exa_udf_entry`. Mirror `scalar-double` crate layout/Cargo.toml.
6. Scaffold `test-udfs/timestamp-now` (emits local wall-clock now via `chrono::Local`/`time`-tz so a named `TZ` resolves from zoneinfo); cdylib for musl. Verify the chosen crate resolves a named zone on a fully-static musl binary (build the `.so`, inspect that it does not statically embed only UTC). [expert]
7. Scaffold `test-udfs/timestamp-passthrough` (reads `Value::Timestamp`, emits unchanged) with a `#[cfg(test)]` `TestCtx` test asserting nanosecond pass-through at the `Value` level; cdylib for musl.
8. Wire the three new UDF libs into the workspace and into the CI artifact build that produces the `.so` files consumed by the `it` harness (match how `scalar-double` etc. are built per `EXAUDF_PARSER_VERSION`/musl target).
9. Add `const TS_ADD_LIB`, `TS_NOW_LIB`, `TS_PASS_LIB` filename constants to `crates/it/tests/db_roundtrip.rs`, upload them via `read_udf_artifact`, and add three new scenario functions invoked in order inside `db_roundtrip_all_scenarios`: `timestamp_arithmetic_roundtrips`, `udf_local_time_matches_session_tz`, `timestamp_precision_matrix_roundtrips`. [expert]
10. In `udf_local_time_matches_session_tz`, run `ALTER SESSION SET TIME_ZONE='Europe/Berlin'` (verify exact accepted zone-name syntax against the running DB), query `udf_now()` and `CURRENT_TIMESTAMP`, assert agreement within a few-seconds tolerance AND non-UTC Berlin offset; document the broken-image UTC behavior as the regression gate. [expert]
11. In `timestamp_precision_matrix_roundtrips`, register `ts_pass_p` for each `p` ∈ {0,3,6,9} (one `RETURNS TIMESTAMP(p)` script each over the same `.so`), round-trip a `p`-digit literal, and assert exact preserved precision against what the DB stores for the same literal. Note in a comment that `p=9` is the regression case that fails under the old `%.6f` and passes under `%.9f`, and that correctness relies on the engine truncating the emitted 9 digits to the column precision on receipt. [expert]
12. Bump workspace `version` in `Cargo.toml` (`0.12.0` → `0.12.1`).
13. Run the verification checklist (build, test, clippy, fmt) and the `db-roundtrip` suite against a local `exasol/docker-db` container.
14. Open a PR against `main` with a `fix:` Conventional Commit title referencing INTEGRATION_POINTS_REPORT gap #1.

## Parallelization

| Parallel Group | Tasks |
|----------------|-------|
| Group A (independent) | Task 2 (Dockerfile), Task 3+4 (rowset emit + unit test, same file), Task 5 (add-second), Task 6 (now), Task 7 (passthrough), Task 12 (version bump) |
| Group B (after A) | Task 8 (workspace/CI wiring), Task 9–11 (e2e scenarios) |
| Group C (after B) | Task 13 (verify), Task 14 (PR) |

Sequential dependencies:
- Task 1 → Group A → Group B → Group C
- Tasks 3 and 4 edit the same file (`rowset.rs`) and MUST be done together by one worker (standard task — a one-line format change plus its unit test).
- Tasks 9–11 edit the same file (`db_roundtrip.rs`) and depend on Tasks 5–7 (the UDF crates) being built (Task 8).

## Dead Code Removal

None. The `TIMESTAMP_EMIT` constant is retained, with its value changed from `%.6f` to `%.9f`; no symbols are removed.

## Verification

### Scenario Coverage

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| Alpine runtime stage is slim and self-sufficient (CHANGED) | Integration | `crates/it` slim-image image-inspection test (existing build-and-inspect harness) | image build asserts `tzdata` installed |
| Runtime image bundles the IANA zoneinfo database | Integration | `crates/it` slim-image image-inspection test | asserts `/usr/share/zoneinfo/Europe/Berlin` present in image |
| EmitBuffer emits timestamps at full nanosecond precision | Unit | `crates/exa-udf-runtime/src/rowset.rs` (`#[cfg(test)] mod tests`) | `timestamp_emit_nanosecond_roundtrip` |
| timestamp-add-second adds one second to a TIMESTAMP input | Unit | `test-udfs/timestamp-add-second/src/lib.rs` (`TestCtx`) | `adds_one_second` |
| timestamp-now emits local wall-clock time in the session timezone | Integration | `crates/it/tests/db_roundtrip.rs` | `udf_local_time_matches_session_tz` |
| timestamp-passthrough reads and re-emits a TIMESTAMP unchanged | Unit | `test-udfs/timestamp-passthrough/src/lib.rs` (`TestCtx`) | `passes_nanosecond_timestamp_through` |
| Test UDF .so builds for the musl target (3 new crates) | Integration | `crates/it/tests/db_roundtrip.rs` (artifact upload) | covered by `read_udf_artifact` of each new lib |
| Timestamp arithmetic round-trips through a SCALAR UDF | Integration | `crates/it/tests/db_roundtrip.rs` | `timestamp_arithmetic_roundtrips` |
| UDF local time agrees with the session timezone and is not UTC | Integration | `crates/it/tests/db_roundtrip.rs` | `udf_local_time_matches_session_tz` |
| TIMESTAMP fractional precision round-trips through the engine's truncation for the 0/3/6/9 matrix | Integration | `crates/it/tests/db_roundtrip.rs` | `timestamp_precision_matrix_roundtrips` |

Unit tests for the rowset precision logic and the UDF crate bodies are justified: they are pure computation over `Value`/`NaiveDateTime` with no I/O. Timezone resolution and DB precision metadata are NOT unit-testable on the host — they require the real static-musl binary inside the container, hence the integration scenarios.

### Manual Testing

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| runtime/dispatch-run-loop | `cargo test -p exa-udf-runtime rowset::tests` | precision unit test passes; a nanosecond timestamp emits 9 digits and round-trips losslessly |
| examples/test-udfs | `cargo build --release --target x86_64-unknown-linux-musl -p timestamp-add-second -p timestamp-now -p timestamp-passthrough` | three `lib*.so` artifacts produced, each exporting `__exa_udf_entry` |
| container/slim-image | `docker build -f Dockerfile.alpine -t slc-rs-slim:dev . && docker run --rm slc-rs-slim:dev ls /usr/share/zoneinfo/Europe/Berlin` | path exists (exit 0) |
| integration/db-roundtrip | `cargo test -p it db_roundtrip_all_scenarios -- --ignored --nocapture` | new timestamp/timezone scenarios print `ok`; `udf_now` reports Berlin offset, not UTC |

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Build | `cargo build --release` | Exit 0 |
| Test | `cargo test` | 0 failures |
| Lint | `cargo fmt --check && cargo clippy --all-targets --all-features -- -D warnings` | 0 errors/warnings |
| Format | `cargo fmt --check` | No changes |
