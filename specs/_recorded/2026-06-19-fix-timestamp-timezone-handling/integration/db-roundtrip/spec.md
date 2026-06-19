# Feature: db-roundtrip

Exercises the full Rust SLC against a live Exasol container — registering the slim SLC, uploading UDF artifacts to BucketFS, and invoking UDFs end-to-end to prove the wire protocol, dispatch, and SDK behave correctly against a real database.

## Background

The integration harness starts an `exasol/docker-db:<version>` container, registers the slim SLC, and uploads UDF `.so` artifacts to BucketFS. All scenarios share one container and run sequentially inside `db_roundtrip_all_scenarios`. This delta adds three timestamp scenarios — including the regression test for the `tzdata` packaging fix, which reports UTC on an image without `tzdata` and the correct Berlin offset with it.

## Scenarios

<!-- DELTA:NEW -->
### Scenario: Timestamp arithmetic round-trips through a SCALAR UDF

* *GIVEN* the `timestamp-add-second` UDF `.so` uploaded and a `CREATE OR REPLACE RUST SCALAR SCRIPT ts_add_second(t TIMESTAMP) RETURNS TIMESTAMP` referencing its `%udf_object` path
* *WHEN* the harness runs `SELECT ts_add_second(TIMESTAMP '2026-06-14 09:30:15.250000')` against the live `exasol/docker-db` container
* *THEN* the query MUST return a TIMESTAMP equal to the input plus exactly one second (`2026-06-14 09:30:16.250000`)
* *AND* the returned value MUST match the input's sub-second component, proving the timestamp survives the decode/emit round-trip rather than being zeroed or truncated

### Scenario: UDF local time agrees with the session timezone and is not UTC

* *GIVEN* the `timestamp-now` UDF `.so` uploaded and a `CREATE OR REPLACE RUST SCALAR SCRIPT udf_now() RETURNS TIMESTAMP` referencing its `%udf_object` path
* *AND* the session timezone set to a named non-UTC zone via `ALTER SESSION SET TIME_ZONE='Europe/Berlin'`
* *WHEN* the harness runs `SELECT udf_now()` and `SELECT CURRENT_TIMESTAMP` on the same session
* *THEN* the UDF wall-clock value MUST agree with the DB `CURRENT_TIMESTAMP` within a bounded skew tolerance of a few seconds (covering execution latency)
* *AND* the UDF wall-clock value MUST NOT equal the UTC wall-clock time for the same instant — its offset MUST be the Berlin offset (`+01:00` or `+02:00` depending on DST), proving the named zone resolved from the bundled `tzdata`
* *AND* this scenario is the regression test for the `tzdata` packaging fix: it MUST fail (UDF reports UTC) against an Alpine image built without `tzdata` and MUST pass against an image with `tzdata`

### Scenario: TIMESTAMP fractional precision round-trips through a UDF at the microsecond input cap for the 0/3/6/9 matrix

* *GIVEN* the `timestamp-passthrough` UDF `.so` uploaded
* *AND* for each precision `p` in {0, 3, 6, 9} a `CREATE OR REPLACE RUST SCALAR SCRIPT ts_pass_p(t TIMESTAMP(p)) RETURNS TIMESTAMP(p)` referencing its `%udf_object` path (verifying the exact `TIMESTAMP(p)` registration syntax the DB accepts)
* *WHEN* the harness runs each `ts_pass_p` over a literal carrying `p` significant fractional digits (e.g. `123456789` truncated to `p`)
* *THEN* each returned value MUST equal the input literal truncated to `min(p, 6)` fractional digits and widened back to `TIMESTAMP(p)` — because the DB delivers every UDF *input* column at microsecond precision (`SWIGTableData::getTimestamp` formats `...FF6`, verified in the engine source and empirically across Rust, Python, and Java), so the realistic round-trip ceiling through any UDF is microseconds
* *AND* for `p` in {0, 3, 6} this is exactly lossless — equal to what the DB stores for the same `TIMESTAMP(p)` literal — since the literal carries no precision beyond microseconds to lose
* *AND* `p = 9` is the input-cap case: the UDF receives `2026-06-14 09:30:15.123456000` (digits 7–9 dropped on the input wire, not by the SLC), so the round-trip returns `.123456000`, NOT the stored literal `.123456789`; the `%.9f` emit format cannot recover precision the input wire already discarded (the emit format's nanosecond capability benefits only UDF-*generated* sub-microsecond values, proven separately by the `timestamp_emit_nanosecond_roundtrip` unit test)
* *AND* `p = 0` MUST NOT be padded with a spurious `.000000`, the engine having truncated the emitted fraction away
<!-- /DELTA:NEW -->
