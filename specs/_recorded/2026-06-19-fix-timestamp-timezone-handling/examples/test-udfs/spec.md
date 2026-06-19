# Feature: test-udfs

Provides the canonical example UDF crates that demonstrate each SDK capability and serve as fixtures for the integration tests.

## Background

Each example is a standalone cdylib crate depending on `exasol-udf-sdk` (plus the macro) and builds for the `x86_64-unknown-linux-musl` target. This delta adds three timestamp fixtures used by the `db-roundtrip` suite to prove timestamp arithmetic, named-timezone resolution, and precision-preserving round-trips.

## Scenarios

<!-- DELTA:NEW -->
### Scenario: timestamp-add-second adds one second to a TIMESTAMP input

* *GIVEN* the `timestamp-add-second` crate with a `#[exasol_udf]` struct implementing `UdfRun`
* *WHEN* its `run` reads the first column as a `Value::Timestamp(NaiveDateTime)` and emits that timestamp plus one second as `Value::Timestamp`
* *THEN* the crate MUST compile to a cdylib for the `x86_64-unknown-linux-musl` target exporting `__exa_udf_entry`
* *AND* a `#[cfg(test)]` `TestCtx` test MUST assert that for input `2026-06-14 09:30:15.250000` the emitted value is `2026-06-14 09:30:16.250000`
* *AND* a NULL input MUST emit `Value::Null`

### Scenario: timestamp-now emits the local wall-clock time in the session timezone

* *GIVEN* the `timestamp-now` crate with a `#[exasol_udf]` struct implementing `UdfRun`
* *WHEN* its `run` reads the current local wall-clock time (resolving the `TZ` env via the zoneinfo database) and emits it as a `Value::Timestamp` whose naive value reflects that local time
* *THEN* the crate MUST compile to a cdylib for the `x86_64-unknown-linux-musl` target exporting `__exa_udf_entry`
* *AND* the implementation MUST obtain local time through a mechanism (`chrono::Local` or `time` with a tz feature) that consults the IANA zoneinfo database for a named `TZ`, so the emitted wall-clock value is correct when `tzdata` is present and would be UTC when it is absent
* *AND* the timezone resolution MUST be proven by the `integration/db-roundtrip` end-to-end scenario, not by a unit test, because zoneinfo resolution on a static-musl binary cannot be exercised in a host unit test

### Scenario: timestamp-passthrough reads and re-emits a TIMESTAMP unchanged

* *GIVEN* the `timestamp-passthrough` crate with a `#[exasol_udf]` struct implementing `UdfRun`
* *WHEN* its `run` reads the first column as `Value::Timestamp` and emits it unchanged
* *THEN* the crate MUST compile to a cdylib for the `x86_64-unknown-linux-musl` target exporting `__exa_udf_entry`
* *AND* a `#[cfg(test)]` `TestCtx` test MUST assert that a nanosecond-resolution timestamp (e.g. `2026-06-14 09:30:15.123456789`) passes through unchanged at the SDK `Value` level
* *AND* this UDF MUST serve as the precision-matrix fixture, so the declared output precision (set by the `RETURNS TIMESTAMP(p)` registration), not the UDF body, governs the wire-emitted fractional digits
<!-- /DELTA:NEW -->
