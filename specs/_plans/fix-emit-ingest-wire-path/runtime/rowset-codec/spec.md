# Feature: rowset-codec

Packs and unpacks UDF row values against the wire's row-major proto type blocks — `EmitBuffer` (row-based output encoding, flush-threshold byte accounting, full-precision timestamp formatting) and `InputRowSet` (row-major decode) — and specifies the promoted fast-path formatter/parser that may replace the `chrono`/`Display`-based implementation without changing wire bytes. Driven by `runtime/dispatch-run-loop`, which owns the scalar/set dispatch loop and calls into this codec to materialise input rows and buffer/flush emitted output; this feature specifies the row-based codec's packing, flushing, and byte-identity guarantees in isolation from that driving loop. The opt-in Arrow batch-emit path (`push_batch`, `emit_batch`) that encodes a whole `RecordBatch` column-at-a-time is specified separately in `runtime/emit-arrow-batch`.

## Background

The rowset codec (`InputRowSet`/`EmitBuffer`) packs output values by declared column `ExaType` rather than by runtime `Value` variant. The decode path parses TIMESTAMP via `%.f` (0..9 fractional digits, lossless), but the emit path historically hardcoded exactly 6 fractional digits (`%.6f`) — capping `TIMESTAMP(7/8/9)` columns at microseconds. The Exasol engine truncates an emitted timestamp to the output column's declared precision on receipt (`SWIGResultHandler::setTimestamp` parses `YYYY-MM-DD HH24:MI:SS.FF9` and applies `trunc_to_fractional_seconds_precision(value, m_types[col].prec)`, verified in `../db/Engine/src/exscript/pluggable/swigcontainers_int.h:1064-1082` and `zmqcontainer.cc:675`). Therefore emitting MORE fractional digits than the column declares is safe (the engine truncates); emitting FEWER loses precision. This delta makes the emit always carry the full available nanosecond precision (`%.9f`) so the engine's own truncation yields the exact declared precision — the SLC does not truncate client-side and does not need the output column metadata threaded into the encoder. This concerns the **emit/output** path only; it lets UDF-*generated* sub-microsecond values (wall-clock, connect-back data) reach an output column at up to nanosecond precision. It does NOT widen UDF *input*: the engine delivers every input column at microsecond precision (`SWIGTableData::getTimestamp` formats `...FF6`, `swigcontainers_int.h:779-781`), so an input→output round-trip through a UDF is capped at microseconds regardless of this emit format.

The exact wire-format strings the Exasol engine parses are fixed contracts: `DATE_FORMAT = "%Y-%m-%d"`, `TIMESTAMP_EMIT = "%Y-%m-%d %H:%M:%S%.9f"` (full nanosecond precision, engine-truncated to the declared column precision), and fixed-point decimal via `Decimal`'s `Display`. Any performance optimisation of the formatting/parsing path — whether a hand-rolled fast formatter or a fast decimal/date parser — must leave those wire bytes and the `EMIT_BUFFER_LIMIT_BYTES` (`4_000_000`) flush semantics unchanged.

<!-- DELTA:NEW -->
<!-- /DELTA:NEW -->

## Scenarios

<!-- DELTA:NEW -->
### Scenario: Emit stamps the source input row number for pass-through columns

* *GIVEN* a loaded UDF whose input axis is `ExactlyOnce` (SCALAR) and whose `MT_NEXT` batch carries the engine's `ExascriptTableData.row_number` array, one monotonic entry per input row
* *WHEN* the UDF emits rows through `emit` or returns a value through `set_return` while an input row is current
* *THEN* `InputRowSet` MUST retain the batch's `row_number` array and MUST expose the current row's number
* *AND* the bridge MUST stamp the current input row's number onto each output row it buffers, so a scalar invocation that emits many rows stamps the same number on all of them
* *AND* `to_proto` MUST serialise one `row_number` entry per emitted row, in the same order as the rows, instead of the empty array it sends today
* *AND* for input axis `Multiple` (SET) the emitted `row_number` array MUST stay empty
* *AND* when an input batch supplies fewer `row_number` entries than rows, the emitted array MUST stay empty
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: InputRowSet materialises one row at a time from the proto blocks

* *GIVEN* an `ExascriptTableData` batch and its `ColumnMeta`
* *WHEN* the runtime decodes the batch and walks its rows with `advance`
* *THEN* `from_proto` MUST take the table by value and MUST drain `data_string` with `into_iter`, so each string moves out of the proto array instead of being cloned
* *AND* `InputRowSet` MUST keep the proto typed arrays as its storage and MUST materialise only the current row into one reusable scratch `Vec<Value>`, so decoding a batch of N rows performs no per-row heap allocation
* *AND* `get` MUST keep returning `&Value` and `current_row` MUST keep returning `&[Value]`, both borrowed from that scratch buffer
* *AND* each decoded `Value` MUST equal the value the current dense-matrix decode produces for the same wire bytes, preserving the per-type cursor rule that a NULL cell consumes no type-block slot
* *AND* the random-access `row(idx)` accessor MUST be replaced by a forward-only `seek_row(idx)` that positions on `idx` when `idx` is at or after the current row and returns `None` otherwise
<!-- /DELTA:NEW -->

<!-- DELTA:CHANGED -->
### Scenario: EmitBuffer tracks a running byte estimate and reports when to flush

* *GIVEN* a fresh `EmitBuffer`
* *WHEN* rows are appended via `push`
* *THEN* `push` MUST increase a `byte_estimate` field by an approximation of the wire size of the pushed values (summing per-value byte costs), and `should_flush` MUST return true exactly when `byte_estimate` is greater than or equal to `EMIT_BUFFER_LIMIT_BYTES` (`4_000_000`)
* *AND* `clear` MUST reset both the row vector and the `byte_estimate` to zero so a flushed buffer starts a fresh accounting cycle
* *AND* the byte estimate MUST be a monotonic non-negative running total computed without re-serializing the whole buffer on every `push`, so emit cost stays linear in the number of rows
* *AND* a NUMERIC cell's cost MUST equal the exact byte length of the decimal string `to_proto` writes for it, computed in O(1) from the digit count `d` of the unscaled magnitude (`checked_ilog10` plus one, and one for a zero magnitude), the scale `s`, and a sign charge `n` of one for a negative value: `n + d` when `s` is zero, `n + d + 1` when `d` exceeds `s`, and `n + 2 + s` otherwise
* *AND* the fixed `NUMERIC_COST_BASE` MUST be removed; the Arrow batch path's `fixed_cell_cost` MUST charge a `Decimal128(precision, scale)` cell `precision + 2` bytes
* *AND* a group-boundary reset MUST clear the rows and the byte estimate while retaining the row vector's allocated capacity, and MUST NOT increment `flush_count`
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: EmitBuffer packs output values row-major by declared column type

* *GIVEN* an `EmitBuffer` holding rows where a column's declared `ExaType` differs from the runtime `Value` variant (e.g. `ExaType::Numeric` with `Value::Int64`)
* *WHEN* `EmitBuffer::to_proto` is called with the declared column metadata
* *THEN* each value MUST be packed into the type block dictated by the declared `ExaType`, not by the `Value` variant — a `Value::Int64` in a `Numeric` column MUST be stringified and written to the string block
* *AND* values for successive columns of the same type within the same row MUST appear contiguously in row-major order within their type block
* *AND* a NULL cell MUST NOT occupy any slot in its type block — only the null-bitmap is updated
* *AND* `to_proto` MUST take `&mut self`, consume the buffered rows out of the buffer, and move each `String` into the proto string block through `value_into_block_string`, so an emitted string is never cloned between the buffer and the proto message
* *AND* the consume MUST leave the buffer holding no rows, so a second `to_proto` before the next `push` MUST produce a zero-row table rather than re-sending the flushed rows
* *AND* the resulting `ExascriptTableData` MUST stay byte-identical to what the cloning encoder produced for every representable value apart from the `row_number` array this delta populates
<!-- /DELTA:CHANGED -->
