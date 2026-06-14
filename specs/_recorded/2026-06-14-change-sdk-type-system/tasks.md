# Tasks: change-sdk-type-system

## Phase 2: Group A — Foundation

- [x] 1.1 Add `chrono = "0.4"` to `[workspace.dependencies]` in `Cargo.toml`; add `exasol-udf-sdk` dep to `exa-zmq-protocol/Cargo.toml`; add `chrono` to `exasol-udf-sdk/Cargo.toml`, `exa-zmq-protocol/Cargo.toml`, `exa-udf-runtime/Cargo.toml`

## Phase 2: Group B — SDK Core

- [x] 2.1 Introduce `Decimal { unscaled: i128, scale: u8 }` newtype in `exasol-udf-sdk/src/value.rs` with `TryFrom<&str>`, `TryFrom<f64>`, `Display` (lossless wire round-trip) [expert]
- [x] 2.2 Rework `Value` enum: `Numeric(Decimal)`, `Date(NaiveDate)`, `Timestamp(NaiveDateTime)`; add canonical extended `ExaType` with all variants (`Char { size }`, `Geometry`, `HashType`, `IntervalYearToMonth`, `IntervalDayToSecond`, `TimestampTz`, plus existing with fields) [expert]

## Phase 2: Group C — Consumers

- [x] 3.1 Remove duplicate `ExaType` from `exa-zmq-protocol/src/meta.rs`; re-export SDK enum; update `ColumnMeta` accordingly [expert]
- [x] 3.2 Implement `type_name` refinement in `ColumnMeta::from_pb` (CHAR/VARCHAR, GEOMETRY/HASHTYPE/INTERVAL on PB_STRING, TIMESTAMP WITH LOCAL TIME ZONE on PB_TIMESTAMP) and inverse mapping in `ColumnMeta::to_pb`
- [x] 3.3 Add typed getters (`get_i64`, `get_f64`, `get_string`, `get_bool`, `get_decimal`, `get_date`, `get_timestamp`, `get_value`) plus `reset`/column-introspection methods to `UdfContext` trait, with NULL→`None` and strict type checks (documented `Numeric`→`i64` exception)

## Phase 2: Group D — Downstream

- [x] 4.1 Update `exa-udf-runtime/src/rowset.rs` decode/encode to materialize and serialize typed `Value` variants losslessly across all type blocks (Decimal, Date, Timestamp, extended string-backed types) [expert]
- [x] 4.2 Update `exasol-udf-sdk/src/connect_back.rs` (`cell_to_value`) to produce typed `Value::Numeric(Decimal)`/`Value::Date(NaiveDate)`/`Value::Timestamp(NaiveDateTime)` from arrow cells [expert]
- [x] 4.3 Update `exasol-udf-macros/src/lib.rs` `rust_type_to_exatype` to additionally map `Decimal`, `NaiveDate`, `NaiveDateTime`

## Phase 2: Group E — Callers + Docs

- [x] 5.1 Update all `test-udfs/*` that read or emit `Value` (scalar-double, set-filter, json-parse, annotated-*, connect-back-*) to the typed API
- [x] 5.2 Add the "## Exasol data type mapping" section to `language-container-rs/CLAUDE.md` (already done as part of conflict resolution)

## Phase 3: Verification

- [x] 6.1 Update unit tests in `value.rs`, `rowset.rs`, `meta.rs`, `context.rs`, `connect_back.rs` to cover new types; run `cargo fmt`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test`
- [x] 6.2 Run integration tests: `cargo +1.91 test -p it --features integration` against live Exasol Docker; 0 failures
