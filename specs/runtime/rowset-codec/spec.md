# Feature: rowset-codec

Packs and unpacks UDF row values against the wire's row-major proto type blocks — `EmitBuffer` (row-based output encoding, flush-threshold byte accounting, full-precision timestamp formatting) and `InputRowSet` (row-major decode) — and specifies the promoted fast-path formatter/parser that may replace the `chrono`/`Display`-based implementation without changing wire bytes. Driven by `runtime/dispatch-run-loop`, which owns the scalar/set dispatch loop and calls into this codec to materialise input rows and buffer/flush emitted output; this feature specifies the row-based codec's packing, flushing, and byte-identity guarantees in isolation from that driving loop. The opt-in Arrow batch-emit path (`push_batch`, `emit_batch`) that encodes a whole `RecordBatch` column-at-a-time is specified separately in `runtime/emit-arrow-batch`.

## Background

The rowset codec (`InputRowSet`/`EmitBuffer`) packs output values by declared column `ExaType` rather than by runtime `Value` variant. The decode path parses TIMESTAMP via `%.f` (0..9 fractional digits, lossless), but the emit path historically hardcoded exactly 6 fractional digits (`%.6f`) — capping `TIMESTAMP(7/8/9)` columns at microseconds. The Exasol engine truncates an emitted timestamp to the output column's declared precision on receipt (`SWIGResultHandler::setTimestamp` parses `YYYY-MM-DD HH24:MI:SS.FF9` and applies `trunc_to_fractional_seconds_precision(value, m_types[col].prec)`, verified in `../db/Engine/src/exscript/pluggable/swigcontainers_int.h:1064-1082` and `zmqcontainer.cc:675`). Therefore emitting MORE fractional digits than the column declares is safe (the engine truncates); emitting FEWER loses precision. This delta makes the emit always carry the full available nanosecond precision (`%.9f`) so the engine's own truncation yields the exact declared precision — the SLC does not truncate client-side and does not need the output column metadata threaded into the encoder. This concerns the **emit/output** path only; it lets UDF-*generated* sub-microsecond values (wall-clock, connect-back data) reach an output column at up to nanosecond precision. It does NOT widen UDF *input*: the engine delivers every input column at microsecond precision (`SWIGTableData::getTimestamp` formats `...FF6`, `swigcontainers_int.h:779-781`), so an input→output round-trip through a UDF is capped at microseconds regardless of this emit format.

The exact wire-format strings the Exasol engine parses are fixed contracts: `DATE_FORMAT = "%Y-%m-%d"`, `TIMESTAMP_EMIT = "%Y-%m-%d %H:%M:%S%.9f"` (full nanosecond precision, engine-truncated to the declared column precision), and fixed-point decimal via `Decimal`'s `Display`. Any performance optimisation of the formatting/parsing path — whether a hand-rolled fast formatter or a fast decimal/date parser — must leave those wire bytes and the `EMIT_BUFFER_LIMIT_BYTES` (`4_000_000`) flush semantics unchanged.

## Scenarios

### Scenario: EmitBuffer packs output values row-major by declared column type

* *GIVEN* an `EmitBuffer` holding rows where a column's declared `ExaType` differs from the runtime `Value` variant (e.g. `ExaType::Numeric` with `Value::Int64`)
* *WHEN* `EmitBuffer::to_proto` is called with the declared column metadata
* *THEN* each value MUST be packed into the type block dictated by the declared `ExaType`, not by the `Value` variant — a `Value::Int64` in a `Numeric` column MUST be stringified and written to the string block
* *AND* values for successive columns of the same type within the same row MUST appear contiguously in row-major order within their type block
* *AND* a NULL cell MUST NOT occupy any slot in its type block — only the null-bitmap is updated

### Scenario: InputRowSet decodes row-major type blocks correctly

* *GIVEN* a `ExascriptTableData` whose type blocks are populated row-major by `EmitBuffer::to_proto` (non-null cells only, per declared column type)
* *WHEN* `InputRowSet::from_proto` decodes the table
* *THEN* it MUST reconstruct the original row/column values by advancing per-type cursors only for non-null cells
* *AND* the decoded rows MUST match the values that were emitted, preserving column types according to the declared metadata

### Scenario: A single emitted row larger than the flush threshold is sent on its own

* *GIVEN* a loaded set UDF whose single emitted row carries a value whose serialized size alone exceeds `EMIT_BUFFER_LIMIT_BYTES` (`4_000_000` bytes)
* *WHEN* the UDF calls `emit` once with that oversized row
* *THEN* the bridge MUST push the whole row into the `EmitBuffer` as one unit and MUST NOT split a single row across `MT_EMIT` frames, because the wire protocol packs rows atomically
* *AND* the bridge MUST then observe that the buffer's byte estimate crosses the threshold and flush the single-row buffer in one `MT_EMIT`, accepting that the frame exceeds the nominal 4,000,000-byte target rather than dropping or truncating the row
* *AND* the only hard ceiling that remains MUST be the protocol's 2 GB per-value limit, which the runtime does not attempt to circumvent

### Scenario: EmitBuffer tracks a running byte estimate and reports when to flush

* *GIVEN* a fresh `EmitBuffer`
* *WHEN* rows are appended via `push`
* *THEN* `push` MUST increase a `byte_estimate` field by an approximation of the wire size of the pushed values (summing per-value byte costs), and `should_flush` MUST return true exactly when `byte_estimate` is greater than or equal to `EMIT_BUFFER_LIMIT_BYTES` (`4_000_000`)
* *AND* `clear` MUST reset both the row vector and the `byte_estimate` to zero so a flushed buffer starts a fresh accounting cycle
* *AND* the byte estimate MUST be a monotonic non-negative running total computed without re-serializing the whole buffer on every `push`, so emit cost stays linear in the number of rows

### Scenario: EmitBuffer emits timestamps at full nanosecond precision

* *GIVEN* an `EmitBuffer` holding a `Value::Timestamp(NaiveDateTime)` carrying sub-microsecond (nanosecond) precision
* *WHEN* `EmitBuffer::to_proto` serialises the row into the string block
* *THEN* the emitted timestamp string MUST contain exactly 9 fractional-second digits (chrono `%.9f`), reproducing the full nanosecond component of the `NaiveDateTime`
* *AND* the emitted string MUST round-trip losslessly: decoding it via `InputRowSet::from_proto` MUST reproduce the original nanosecond-resolution `NaiveDateTime`
* *AND* the previous hardcoded 6-digit emit format (`%.6f`) MUST NOT be used, since it capped output at microseconds and lost precision for `TIMESTAMP(7)`, `TIMESTAMP(8)`, and `TIMESTAMP(9)` columns
* *AND* the encoder MUST NOT consult the output `ColumnMeta` precision: the Exasol engine truncates the emitted value to the column's declared precision on receipt, so emitting all 9 digits is correct for every declared precision (a plain `TIMESTAMP`, which defaults to precision 3, is truncated 9→3 by the engine exactly as it was truncated 6→3 before)

### Scenario: A promoted emit fast-path encoder stays byte-identical to the row path

* *GIVEN* an `EmitBuffer` whose internal formatting of NUMERIC/DATE/TIMESTAMP/VARCHAR cells into the proto string block is produced by a performance-optimised encoder selected after benchmarking (for example a hand-rolled or `itoa`/`ryu`-based formatter replacing `chrono`'s generic `format` / the `Decimal` `Display` impl, or a columnar transport path promoted from a spike)
* *AND* the equivalent rows expressed through the current `chrono`/`Display`-based row path over the same declared `ColumnMeta` output schema
* *WHEN* `EmitBuffer::to_proto` serialises rows spanning the full `ExaType` range — including NULL cells and multiple columns sharing one block type
* *THEN* the resulting `ExascriptTableData` MUST be byte-identical to the output the current `chrono`/`Display`-based row path produces for every representable value, so downstream Exasol parsing — which depends on the exact `%Y-%m-%d` (`DATE_FORMAT`), `%Y-%m-%d %H:%M:%S%.9f` (`TIMESTAMP_EMIT`), and fixed-point decimal format strings — is unaffected
* *AND* the encoder MUST preserve the `EMIT_BUFFER_LIMIT_BYTES` (`4_000_000`) flush semantics unchanged — the running byte estimate, the mid-run threshold flush, and the end-of-`run` tail flush
* *AND* a NULL cell MUST NOT occupy a slot in its type block, and the dense row-major-interleaved block layout `to_proto` produces MUST be preserved exactly

### Scenario: A promoted ingest fast-path decoder round-trips byte-identically

* *GIVEN* an `InputRowSet` whose string-block parsing of NUMERIC/DATE/TIMESTAMP cells is produced by a performance-optimised decoder selected after benchmarking (the symmetric ingest mirror of the promoted emit fast-path), replacing `chrono`'s `parse_from_str` / decimal parsing in `decode_string_block`
* *WHEN* `InputRowSet::from_proto` decodes an `ExascriptTableData` covering the full `ExaType` range including NULL cells
* *THEN* each decoded `Value` MUST equal the `Value` the current `chrono`-based decode path produces for the same wire bytes, preserving lossless round-trip with the emit path
* *AND* the decoder MUST accept every format the emit path can produce — TIMESTAMP with 0..9 fractional-second digits (`%.f`), DATE as `%Y-%m-%d`, and fixed-point DECIMAL — with no loss of precision
* *AND* a NULL cell MUST NOT consume a slot in its type block, preserving the per-type cursor advancement `from_proto` guarantees
