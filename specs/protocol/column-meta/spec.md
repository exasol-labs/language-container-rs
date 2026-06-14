# Feature: column-meta

Defines `ColumnMeta` construction and serialization: mapping proto column types to the canonical `ExaType`, refining ambiguous proto types via `type_name`, and round-tripping extended `ExaType` variants back to proto column types.

## Background

`ColumnMeta` is produced by `ColumnMeta::from_pb` during handshake metadata processing. The Exasol wire protocol uses eight proto column types (`PB_INT32`, `PB_INT64`, `PB_DOUBLE`, `PB_NUMERIC`, `PB_BOOLEAN`, `PB_STRING`, `PB_DATE`, `PB_TIMESTAMP`). Several SQL types collapse onto the same proto type and are disambiguated by inspecting `type_name`. The canonical `ExaType` lives in `exasol-udf-sdk::value`; `exa-zmq-protocol` re-exports it.

## Scenarios

### Scenario: Metadata maps proto column types to ColumnMeta

* *GIVEN* an `MT_META` response containing the eight v1 column types (`PB_INT32`, `PB_INT64`, `PB_DOUBLE`, `PB_NUMERIC`, `PB_BOOLEAN`, `PB_STRING`, `PB_DATE`, `PB_TIMESTAMP`)
* *WHEN* the protocol processes the metadata
* *THEN* it MUST produce a `Vec<ColumnMeta>` preserving column order, name, and type for every column
* *AND* `ColumnMeta::typ` MUST be the canonical `ExaType` re-exported from `exasol-udf-sdk`, NOT a protocol-local duplicate enum
* *AND* it MUST resolve `iter_type` to `IterType::ExactlyOnce` for `PB_EXACTLY_ONCE` and `IterType::Multiple` for `PB_MULTIPLE`

### Scenario: ColumnMeta refines extended Exasol types from type_name

* *GIVEN* an `MT_META` column whose proto `column_type` collapses several SQL types into one wire type
* *WHEN* `ColumnMeta::from_pb` builds the column descriptor
* *THEN* a `PB_STRING` column MUST resolve to `ExaType::Char { size }` when `type_name` begins with `CHAR`, and to `ExaType::String { size }` for `VARCHAR`
* *AND* a `PB_STRING` column MUST resolve to `ExaType::Geometry`, `ExaType::HashType`, `ExaType::IntervalYearToMonth`, or `ExaType::IntervalDayToSecond` when `type_name` names `GEOMETRY`, `HASHTYPE`, `INTERVAL YEAR ... TO MONTH`, or `INTERVAL DAY ... TO SECOND` respectively
* *AND* a `PB_TIMESTAMP` column MUST resolve to `ExaType::TimestampTz` when `type_name` is `TIMESTAMP WITH LOCAL TIME ZONE`, and to `ExaType::Timestamp` otherwise
* *AND* refinement MUST examine `type_name` only when the proto `column_type` is ambiguous; unambiguous proto types (`PB_INT32`, `PB_INT64`, `PB_DOUBLE`, `PB_BOOLEAN`, `PB_DATE`) MUST map directly without consulting `type_name`

### Scenario: Extended ExaType variants round-trip back to proto column types

* *GIVEN* a `ColumnMeta` carrying an extended `ExaType` (`Char`, `Geometry`, `HashType`, `IntervalYearToMonth`, `IntervalDayToSecond`, or `TimestampTz`)
* *WHEN* `ColumnMeta::to_pb` serializes the descriptor
* *THEN* `Char`, `Geometry`, `HashType`, and both `Interval` variants MUST serialize back to `PB_STRING`
* *AND* `TimestampTz` MUST serialize back to `PB_TIMESTAMP`
* *AND* the original `type_name`, `size`, `precision`, and `scale` fields MUST be preserved unchanged so the descriptor survives a `from_pb`/`to_pb` round-trip
