# Feature: column-meta

Defines `ColumnInfo` construction and serialization: mapping proto column types to the canonical `ExaType`, refining ambiguous proto types via `type_name`, and round-tripping extended `ExaType` variants back to proto column types.

## Background

`ColumnInfo` is produced by `column_from_pb` during handshake metadata processing. The Exasol wire protocol uses eight proto column types (`PB_INT32`, `PB_INT64`, `PB_DOUBLE`, `PB_NUMERIC`, `PB_BOOLEAN`, `PB_STRING`, `PB_DATE`, `PB_TIMESTAMP`). Several SQL types collapse onto the same proto type and are disambiguated by inspecting `type_name`. `ColumnInfo` and the canonical `ExaType` both live in `exasol-udf-sdk::value`, because UDF code reads the descriptor through `UdfContext`; `exa-zmq-protocol` re-exports them.

## Scenarios

### Scenario: Metadata maps proto column types to ColumnInfo

* *GIVEN* an `MT_META` response containing the eight v1 column types (`PB_INT32`, `PB_INT64`, `PB_DOUBLE`, `PB_NUMERIC`, `PB_BOOLEAN`, `PB_STRING`, `PB_DATE`, `PB_TIMESTAMP`)
* *WHEN* the protocol processes the metadata
* *THEN* it MUST produce a `Vec<ColumnInfo>` preserving column order, name, and type for every column
* *AND* `ColumnInfo::typ` MUST be the canonical `ExaType` re-exported from `exasol-udf-sdk`, NOT a protocol-local duplicate enum
* *AND* it MUST resolve `iter_type` to `IterType::ExactlyOnce` for `PB_EXACTLY_ONCE` and `IterType::Multiple` for `PB_MULTIPLE`

### Scenario: ColumnInfo refines extended Exasol types from type_name

* *GIVEN* an `MT_META` column whose proto `column_type` collapses several SQL types into one wire type
* *WHEN* `column_from_pb` builds the column descriptor
* *THEN* a `PB_STRING` column MUST resolve to `ExaType::Char { size }` when `type_name` begins with `CHAR` (`size` defaults to 1 when the proto field is absent), and to `ExaType::String { size }` for `VARCHAR` (`size` defaults to 2,000,000 when absent)
* *AND* a `PB_STRING` column with a `type_name` that does not start with `CHAR` MUST resolve to `ExaType::String` (the DB rejects `GEOMETRY`, `HASHTYPE`, and interval types as UDF columns, so they never reach the wire)
* *AND* a `PB_TIMESTAMP` column MUST resolve to `ExaType::Timestamp { precision }`; `precision` MUST be extracted from the parenthesized `(n)` in `type_name` when present (e.g. `TIMESTAMP(6)` → 6), falling back to `col.precision` then default 3; this is necessary because 8.29.x sends `type_name="TIMESTAMP(3)"` with `col.precision=0` for plain `TIMESTAMP`
* *AND* refinement MUST examine `type_name` only when the proto `column_type` is ambiguous; unambiguous proto types (`PB_INT32`, `PB_INT64`, `PB_DOUBLE`, `PB_BOOLEAN`, `PB_DATE`) MUST map directly without consulting `type_name`

### Scenario: Extended ExaType variants round-trip back to proto column types

* *GIVEN* a `ColumnInfo` carrying an extended `ExaType` (`Char`)
* *WHEN* `column_to_pb` serializes the descriptor
* *THEN* `Char` MUST serialize back to `PB_STRING`

* *AND* the original `type_name`, `size`, `precision`, and `scale` fields MUST be preserved unchanged so the descriptor survives a `from_pb`/`to_pb` round-trip
