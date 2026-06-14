# Feature: wire-protocol

Defines the ZMQ wire protocol between the DB and `exaudfclient`, including how proto metadata is decoded into `ColumnMeta`. This delta makes `ColumnMeta` re-use the SDK's canonical `ExaType` and refine extended Exasol types from `type_name`.

## Background

`ColumnMeta::from_pb` maps the eight proto column types but discards the SQL-level distinctions Exasol encodes in `type_name` (CHAR vs VARCHAR, TIMESTAMP vs TIMESTAMP WITH LOCAL TIME ZONE, and INTERVAL/GEOMETRY/HASHTYPE all riding on `PB_STRING`). This delta deduplicates `ExaType` (re-exporting it from `exasol-udf-sdk`) and adds `type_name`-based refinement into the extended `ExaType` variants. The proto/wire format itself does not change. Only the scenarios below change.

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Metadata maps proto column types to ColumnMeta

* *GIVEN* an `MT_META` response containing the eight v1 column types (`PB_INT32`, `PB_INT64`, `PB_DOUBLE`, `PB_NUMERIC`, `PB_BOOLEAN`, `PB_STRING`, `PB_DATE`, `PB_TIMESTAMP`)
* *WHEN* the protocol processes the metadata
* *THEN* it MUST produce a `Vec<ColumnMeta>` preserving column order, name, and type for every column
* *AND* `ColumnMeta::typ` MUST be the canonical `ExaType` re-exported from `exasol-udf-sdk`, NOT a protocol-local duplicate enum
* *AND* it MUST resolve `iter_type` to `IterType::ExactlyOnce` for `PB_EXACTLY_ONCE` and `IterType::Multiple` for `PB_MULTIPLE`
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: ColumnMeta refines extended Exasol types from type_name

* *GIVEN* an `MT_META` column whose proto `column_type` collapses several SQL types into one wire type
* *WHEN* `ColumnMeta::from_pb` builds the column descriptor
* *THEN* a `PB_STRING` column MUST resolve to `ExaType::Char { size }` when `type_name` begins with `CHAR`, and to `ExaType::String { size }` for `VARCHAR`
* *AND* a `PB_STRING` column MUST resolve to `ExaType::Geometry`, `ExaType::HashType`, `ExaType::IntervalYearToMonth`, or `ExaType::IntervalDayToSecond` when `type_name` names `GEOMETRY`, `HASHTYPE`, `INTERVAL YEAR ... TO MONTH`, or `INTERVAL DAY ... TO SECOND` respectively
* *AND* a `PB_TIMESTAMP` column MUST resolve to `ExaType::TimestampTz` when `type_name` is `TIMESTAMP WITH LOCAL TIME ZONE`, and to `ExaType::Timestamp` otherwise
* *AND* refinement MUST examine `type_name` only when the proto `column_type` is ambiguous; unambiguous proto types (`PB_INT32`, `PB_INT64`, `PB_DOUBLE`, `PB_BOOLEAN`, `PB_DATE`) MUST map directly without consulting `type_name`
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: Extended ExaType variants round-trip back to proto column types

* *GIVEN* a `ColumnMeta` carrying an extended `ExaType` (`Char`, `Geometry`, `HashType`, `IntervalYearToMonth`, `IntervalDayToSecond`, or `TimestampTz`)
* *WHEN* `ColumnMeta::to_pb` serializes the descriptor
* *THEN* `Char`, `Geometry`, `HashType`, and both `Interval` variants MUST serialize back to `PB_STRING`
* *AND* `TimestampTz` MUST serialize back to `PB_TIMESTAMP`
* *AND* the original `type_name`, `size`, `precision`, and `scale` fields MUST be preserved unchanged so the descriptor survives a `from_pb`/`to_pb` round-trip
<!-- /DELTA:NEW -->
