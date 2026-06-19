# Feature: dispatch-run-loop

Orchestrates driving the scalar/set run loop over the wire protocol — covering bridge row materialisation, emit buffering, UDF error propagation, and connect-back availability.

## Background

The rowset codec (`InputRowSet`/`EmitBuffer`) packs output values by declared column `ExaType` rather than by runtime `Value` variant. The decode path parses TIMESTAMP via `%.f` (0..9 fractional digits, lossless), but the emit path historically hardcoded exactly 6 fractional digits (`%.6f`) — capping `TIMESTAMP(7/8/9)` columns at microseconds. The Exasol engine truncates an emitted timestamp to the output column's declared precision on receipt (`SWIGResultHandler::setTimestamp` parses `YYYY-MM-DD HH24:MI:SS.FF9` and applies `trunc_to_fractional_seconds_precision(value, m_types[col].prec)`, verified in `../db/Engine/src/exscript/pluggable/swigcontainers_int.h:1064-1082` and `zmqcontainer.cc:675`). Therefore emitting MORE fractional digits than the column declares is safe (the engine truncates); emitting FEWER loses precision. This delta makes the emit always carry the full available nanosecond precision (`%.9f`) so the engine's own truncation yields the exact declared precision — the SLC does not truncate client-side and does not need the output column metadata threaded into the encoder. This concerns the **emit/output** path only; it lets UDF-*generated* sub-microsecond values (wall-clock, connect-back data) reach an output column at up to nanosecond precision. It does NOT widen UDF *input*: the engine delivers every input column at microsecond precision (`SWIGTableData::getTimestamp` formats `...FF6`, `swigcontainers_int.h:779-781`), so an input→output round-trip through a UDF is capped at microseconds regardless of this emit format.

## Scenarios

<!-- DELTA:NEW -->
### Scenario: EmitBuffer emits timestamps at full nanosecond precision

* *GIVEN* an `EmitBuffer` holding a `Value::Timestamp(NaiveDateTime)` carrying sub-microsecond (nanosecond) precision
* *WHEN* `EmitBuffer::to_proto` serialises the row into the string block
* *THEN* the emitted timestamp string MUST contain exactly 9 fractional-second digits (chrono `%.9f`), reproducing the full nanosecond component of the `NaiveDateTime`
* *AND* the emitted string MUST round-trip losslessly: decoding it via `InputRowSet::from_proto` MUST reproduce the original nanosecond-resolution `NaiveDateTime`
* *AND* the previous hardcoded 6-digit emit format (`%.6f`) MUST NOT be used, since it capped output at microseconds and lost precision for `TIMESTAMP(7)`, `TIMESTAMP(8)`, and `TIMESTAMP(9)` columns
* *AND* the encoder MUST NOT consult the output `ColumnMeta` precision: the Exasol engine truncates the emitted value to the column's declared precision on receipt, so emitting all 9 digits is correct for every declared precision (a plain `TIMESTAMP`, which defaults to precision 3, is truncated 9→3 by the engine exactly as it was truncated 6→3 before)
<!-- /DELTA:NEW -->
