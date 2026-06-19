# Decision Log: fix-timestamp-timezone-handling

Date: 2026-06-19

## Interview

**Q1:** The emit path hardcodes 6 fractional-second digits (`rowset.rs:289`, `TIMESTAMP_EMIT = "%Y-%m-%d %H:%M:%S%.6f"`), so a UDF reading `TIMESTAMP(9)` and emitting it back truncates nanoseconds. What scope?
**A1:** Fix emit to be precision-aware (Recommended). Make the wire emit preserve the column's declared fractional precision (up to 9 digits) so `TIMESTAMP(9)` round-trips losslessly. Include the `rowset.rs` code change PLUS the e2e test that proves it.

**Q2:** Should the e2e tests also cover `TIMESTAMP WITH LOCAL TIME ZONE`, or just plain `TIMESTAMP`?
**A2:** Plain TIMESTAMP only (Recommended). Keep the plan focused on the tzdata gap and precision; `TIMESTAMP WITH LOCAL TIME ZONE` is a larger type-model question deferred to later.

**Q3:** For the CURRENT_TIMESTAMP timezone-correctness test, how strict should the comparison against Exasol's `CURRENT_TIMESTAMP` be?
**A3:** Offset match + bounded skew (Recommended). Set a named zone (`Europe/Berlin`), assert the UDF's wall-clock agrees with the DB's `CURRENT_TIMESTAMP` within a few seconds (covers execution latency) AND that it is NOT UTC — proving named-zone resolution works.

## Design Decisions

### [1] Bundle `tzdata` in the runtime image rather than reading TZ in Rust

- **Decision:** Fix the timezone gap purely by packaging — add `tzdata` to the `Dockerfile.alpine` runtime stage. No Rust code reads or interprets `TZ`.
- **Alternatives:** Have the runtime read `TZ` and load zone data itself; ship a curated subset of zones.
- **Rationale:** INTEGRATION_POINTS_REPORT confirms `chrono::Local`/`time` read `TZ` implicitly via the zoneinfo database; the only missing piece is the zone files. Shipping the full `tzdata` is the minimal, standard fix.
- **Promotes to ADR:** yes

### [2] Emit full nanosecond precision (`%.9f`) and let the engine truncate, rather than formatting per-column precision in the SLC

- **Decision:** Change the emit format from `%.6f` to `%.9f` — update `TIMESTAMP_EMIT` constant (`rowset.rs:289`), used by `value_to_block_string` at `rowset.rs:323`. `%.9f` is a valid chrono specifier that always emits exactly 9 fractional digits from the `NaiveDateTime` nanosecond component; the engine truncates to the actual column precision on receipt.
- **Alternatives:** Thread the output `ColumnMeta` precision into the encoder and format per-column with manual fractional formatting (a `match p` table or hand-rolled truncate/zero-pad, since chrono lacks `%.0f`/`%.1f`/etc.). This was the original plan's `[expert]` approach.
- **Rationale:** VERIFIED against `../db/Engine/src/exscript/pluggable/`: `zmqcontainer.cc:675` reads the SLC's emitted `table.data_string(...)` and calls `out.setTimestamp(col, ...)`; `SWIGResultHandler::setTimestamp` (`swigcontainers_int.h:1064-1082`) parses the string with format `YYYY-MM-DD HH24:MI:SS.FF9` (accepting 0-9 fractional digits) and then applies `trunc_to_fractional_seconds_precision(value, m_types[col].prec)`. Emitting MORE digits than the column declares is therefore safe (the engine truncates); emitting FEWER loses precision. The old `%.6f` is buggy only because it caps below 9, losing digits for `TIMESTAMP(7/8/9)`. Emitting `%.9f` unconditionally is correct for every precision and far simpler — no metadata threading, no chrono `%.Nf` limitation, no `p=0` trailing-dot edge case. Plain `TIMESTAMP` defaults to precision 3, so 9→3 truncation preserves its prior behavior (previously 6→3).
- **Promotes to ADR:** yes

- **Correction (2026-06-19, post-verification):** The decision to emit `%.9f` is RIGHT, but the original premise — that it makes a `TIMESTAMP(9)` *round-trip* losslessly (Q1/A1) — is FALSE. The rationale above verified only the **output** path (`setTimestamp`, FF9). The **input** path was not checked: `SWIGTableData::getTimestamp` (`swigcontainers_int.h:779-781`) formats every UDF input column with `YYYY-MM-DD HH24:MI:SS.FF6` — **microseconds**, hardcoded, for ALL script languages. Confirmed empirically: a `TIMESTAMP '...123456789'` literal arrives at the UDF as `.123456000` in Rust, Python (`microsecond=123456`), and Java (`java.sql.Timestamp.getNanos()=123456000`, despite the type holding 9 digits). Exasol stores 9 digits internally (`Timestamp::get_nanosecond`) and Virtual Schemas can read them (adapter-generated `TO_CHAR(col,'FF9')`, DB2 VS #38, 8.32+), but the UDF input wire is a separate, microsecond-capped path with no per-UDF override. **Net:** `%.9f` benefits only UDF-*generated* sub-microsecond values (wall-clock, connect-back); an input→output round-trip through a UDF is capped at microseconds. The `timestamp_precision_matrix` e2e asserts this corrected expectation (p≤6 lossless; p=9 → `.123456000`).

### [3] Output TIMESTAMP precision lives in the proto `precision` field, not `scale` (and the SLC does not read it)

- **Decision:** No SLC code reads the output column's fractional precision at all (superseded by decision [2]). For the record: the precision the engine applies is `m_types[col].prec`, surfaced over the wire in the proto `precision` field — NOT `scale`. The planning-time guess `col.scale.or(col.precision)` was WRONG.
- **Alternatives:** Read `scale`; read `scale.or(precision)`.
- **Rationale:** VERIFIED against `../db/Engine/src/exscript/pluggable/`: `swigcontainers_int.h:1080` (input write path, `setTimestamp`) and `:777` (output read path) both truncate/check using `m_types[col].prec`; `zmqcontainer.cc:305/317` set the proto `precision` field from `inputColumnPrecision`/`outputColumnPrecision` (which return `.prec`). `scale` is unrelated for TIMESTAMP. Since the engine truncates on receipt (decision [2]), the SLC needs none of this — but the record corrects the original `scale` assumption so future planners do not reintroduce it.
- **Promotes to ADR:** no

### [4] Encoder signature stays `value_to_block_string(v)` — no `ColumnMeta` threading

- **Decision:** Keep the encode helper signature `value_to_block_string(v)`; do not pass the output `&ColumnMeta`.
- **Alternatives:** Thread `&ColumnMeta` into the helper to format the timestamp branch per-column (the original plan).
- **Rationale:** With the engine truncating on receipt (decision [2]), the encoder has no precision-dependent behavior, so there is nothing to thread. Keeping the signature unchanged minimizes the diff and avoids touching `to_proto`'s call site.
- **Promotes to ADR:** no

### [5] Prove timezone resolution only via the real-DB e2e regression test

- **Decision:** The `udf_now()` scenario (Berlin offset, not UTC) is the sole proof of named-zone resolution; no host unit test attempts it.
- **Alternatives:** A host unit test setting `TZ` and asserting `chrono::Local`.
- **Rationale:** Zoneinfo resolution on a fully-static musl `.so` inside the sandbox cannot be reproduced by a host unit test; only the in-container binary against a real session timezone is meaningful. The broken-vs-fixed assertion makes it a true regression gate for the `tzdata` fix.
- **Promotes to ADR:** yes

### [6] Defer `TIMESTAMP WITH LOCAL TIME ZONE`

- **Decision:** E2E coverage is plain `TIMESTAMP` only.
- **Alternatives:** Cover `TIMESTAMP WITH LOCAL TIME ZONE` now.
- **Rationale:** Per interview Q2, the TZ-typed value model is a larger change; keeping scope on the tzdata gap and precision avoids coupling two unrelated efforts.
- **Promotes to ADR:** no

## Review Findings

<!-- Populated by speq-implement after code review. -->
