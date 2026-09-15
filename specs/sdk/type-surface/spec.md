# Type surface

Owns the set of SQL types a Rust UDF can ingest and emit, the per-type limits, and the wire-block mapping.

## Supported input types

| SQL type | Wire block | `ExaType` | `Value` |
|----------|-----------|-----------|---------|
| `DECIMAL(1..9, 0)`, `TINYINT`, `SMALLINT` | `PB_INT32` | `Int32` | `Int32` |
| `DECIMAL(10..18, 0)`, `INTEGER` | `PB_INT64` | `Int64` | `Int64` |
| `DECIMAL(19..36, 0)`, `BIGINT`, any scale > 0 | `PB_NUMERIC` | `Numeric { precision, scale }` | `Numeric` |
| `DOUBLE`, `FLOAT`, `REAL` | `PB_DOUBLE` | `Double` | `Double` |
| `BOOLEAN` | `PB_BOOLEAN` | `Boolean` | `Bool` |
| `VARCHAR(n)` | `PB_STRING` | `String { size }` | `String` |
| `CHAR(n)` | `PB_STRING` | `Char { size }` | `String` |
| `DATE` | `PB_DATE` | `Date` | `Date` |
| `TIMESTAMP(p)` | `PB_TIMESTAMP` | `Timestamp { precision }` | `Timestamp` |
| `TIMESTAMP(p) WITH LOCAL TIME ZONE` | `PB_TIMESTAMP` | `TimestampTz { precision }` | `Timestamp` |

## Ingest-only types

`TIMESTAMP WITH LOCAL TIME ZONE` is accepted as input but rejected by the DB as an output column (SQL state 22002). The SDK rejects it at emit validation time.

## Rejected types

The DB rejects these as UDF columns before values reach the wire:

* Output only: `GEOMETRY`, `HASHTYPE`, `INTERVAL YEAR TO MONTH`, `INTERVAL DAY TO SECOND`
* Both directions: `TIME`, `ARRAY`

## Per-type limits

* `DECIMAL` maximum precision: 36.
* `VARCHAR` maximum: 2,000,000 characters. `CHAR` maximum: 2,000.
* Emitting a value exceeding the column's declared size is a DB-side data exception.
* Emitting an empty string yields NULL (Exasol treats `''` as NULL).

## Timestamp precision

* Input: the DB delivers all UDF inputs at microsecond precision regardless of the column's declared `TIMESTAMP(p)`.
* Output: the SDK emits 9 fractional digits; the engine truncates to the declared precision on receipt.
* A UDF round-trip is therefore capped at microsecond precision.

## Emit buffer

`EMIT_BUFFER_LIMIT_BYTES` (4,000,000) is a flush target, not a DB-enforced wire limit. The DB accepts larger `MT_EMIT` messages.
