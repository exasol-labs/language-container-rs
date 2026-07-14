# Feature: test-udfs-timestamps

Provides the timestamp fixture UDF crates that prove timestamp arithmetic, named-timezone resolution, and precision-preserving round-trips through the SDK. Split from `examples/test-udfs` to keep each feature spec under the ten-scenario threshold.

## Background

Each fixture is a standalone cdylib crate depending only on `exasol-udf-sdk` and builds for the `x86_64-unknown-linux-musl` target. The three crates exercise the `Value::Timestamp` / `Value::TimestampTz` handling end-to-end and are consumed by the live-DB integration suite. All three are SCALAR `RETURNS TIMESTAMP` UDFs, so they produce output by returning `Result<Option<Value>, UdfError>` (the value-return channel), not by calling `ctx.emit`.

## Scenarios

### Scenario: timestamp-add-second adds one second to a TIMESTAMP input

* *GIVEN* the `timestamp-add-second` crate with a `#[exasol_udf]` function whose return type is `Result<Option<Value>, UdfError>`
* *WHEN* its `run` reads the first column as a `Value::Timestamp(NaiveDateTime)` and returns that timestamp plus one second as `Ok(Some(Value::Timestamp(...)))`
* *THEN* the crate MUST compile to a cdylib for the `x86_64-unknown-linux-musl` target exporting the named entry point with the RETURNS output-shape marker
* *AND* a `#[cfg(test)]` `TestCtx` test MUST assert that for input `2026-06-14 09:30:15.250000` the returned value is `2026-06-14 09:30:16.250000`
* *AND* a NULL input MUST return `Ok(None)` (SQL NULL)

### Scenario: timestamp-now returns the local wall-clock time in the session timezone

* *GIVEN* the `timestamp-now` crate with a `#[exasol_udf]` function whose return type is `Result<Option<Value>, UdfError>`
* *WHEN* its `run` reads the current local wall-clock time (resolving the `TZ` env via the zoneinfo database) and returns it as `Ok(Some(Value::Timestamp(...)))` whose naive value reflects that local time
* *THEN* the crate MUST compile to a cdylib for the `x86_64-unknown-linux-musl` target exporting the named entry point with the RETURNS output-shape marker
* *AND* the implementation MUST obtain local time through a mechanism (`chrono::Local` or `time` with a tz feature) that consults the IANA zoneinfo database for a named `TZ`, so the returned wall-clock value is correct when `tzdata` is present and would be UTC when it is absent
* *AND* the timezone resolution MUST be proven by the live-DB integration end-to-end scenario, not by a unit test, because zoneinfo resolution on a static-musl binary cannot be exercised in a host unit test

### Scenario: timestamp-passthrough reads and re-returns a TIMESTAMP unchanged

* *GIVEN* the `timestamp-passthrough` crate with a `#[exasol_udf]` function whose return type is `Result<Option<Value>, UdfError>`
* *WHEN* its `run` reads the first column as `Value::Timestamp` and returns it unchanged as `Ok(Some(Value::Timestamp(...)))`
* *THEN* the crate MUST compile to a cdylib for the `x86_64-unknown-linux-musl` target exporting the named entry point with the RETURNS output-shape marker
* *AND* a `#[cfg(test)]` `TestCtx` test MUST assert that a nanosecond-resolution timestamp (e.g. `2026-06-14 09:30:15.123456789`) passes through unchanged at the SDK `Value` level
* *AND* this UDF MUST serve as the precision-matrix fixture, so the declared output precision (set by the `RETURNS TIMESTAMP(p)` registration), not the UDF body, governs the wire-emitted fractional digits
