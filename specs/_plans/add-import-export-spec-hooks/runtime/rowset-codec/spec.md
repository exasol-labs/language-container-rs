# Feature: rowset-codec

Packs and unpacks UDF row values against the wire's row-major proto type blocks — `EmitBuffer` (row-based output encoding into the blocks themselves, flush-threshold byte accounting, full-precision timestamp formatting) and `InputRowSet` (row-major decode) — and specifies the promoted fast-path formatter/parser that may replace the `chrono`/`Display`-based implementation without changing wire bytes. Driven by `runtime/dispatch-run-loop`, which owns the scalar/set dispatch loop and calls into this codec to materialise input rows and buffer/flush emitted output; this feature specifies the row-based codec's packing, flushing, and byte-identity guarantees in isolation from that driving loop. The opt-in Arrow batch-emit path (`push_batch`, `emit_batch`) that encodes a whole `RecordBatch` column-at-a-time is specified separately in `runtime/emit-arrow-batch`.

<!-- DELTA:CHANGED -->
## Background

The rowset codec (`InputRowSet`/`EmitBuffer`) packs output values by declared column `ExaType` rather than by runtime `Value` variant. The decode path parses TIMESTAMP via `%.f` (0..9 fractional digits, lossless), but the emit path historically hardcoded exactly 6 fractional digits (`%.6f`) — capping `TIMESTAMP(7/8/9)` columns at microseconds. The Exasol engine truncates an emitted timestamp to the output column's declared precision on receipt (`SWIGResultHandler::setTimestamp` parses `YYYY-MM-DD HH24:MI:SS.FF9` and applies `trunc_to_fractional_seconds_precision(value, m_types[col].prec)`, verified in `../db/Engine/src/exscript/pluggable/swigcontainers_int.h:1064-1082` and `zmqcontainer.cc:675`). Therefore emitting MORE fractional digits than the column declares is safe (the engine truncates); emitting FEWER loses precision. This delta makes the emit always carry the full available nanosecond precision (`%.9f`) so the engine's own truncation yields the exact declared precision — the SLC does not truncate client-side and does not need the output column metadata threaded into the encoder. This concerns the **emit/output** path only; it lets UDF-*generated* sub-microsecond values (wall-clock, connect-back data) reach an output column at up to nanosecond precision. It does NOT widen UDF *input*: the engine delivers every input column at microsecond precision (`SWIGTableData::getTimestamp` formats `...FF6`, `swigcontainers_int.h:779-781`), so an input→output round-trip through a UDF is capped at microseconds regardless of this emit format.

`ExascriptTableData.row_number` ("Local row numbers", field 9) numbers the rows of a batch. The engine numbers every input row and pairs an emitted row with its input row by that number: a select-list column the query did not emit but tunnels through the UDF (`SELECT id, f(x) FROM t`) is filled from the input row the emitted row names. The reference C++ SLC therefore adds the current input row's number to every emitted row. Emitting with an empty `row_number` makes the engine read past the list: observed as wrong pass-through values or a closed SQL session. The codec carries the number through: `InputRowSet` keeps the batch's list and `EmitBuffer` records one entry per buffered row.

The exact wire-format strings the Exasol engine parses are fixed contracts: `DATE_FORMAT = "%Y-%m-%d"`, `TIMESTAMP_EMIT = "%Y-%m-%d %H:%M:%S%.9f"` (full nanosecond precision, engine-truncated to the declared column precision), and fixed-point decimal via `Decimal`'s `Display`. Any performance optimisation of the formatting/parsing path — whether a hand-rolled fast formatter or a fast decimal/date parser — must leave those wire bytes and the `EMIT_BUFFER_LIMIT_BYTES` (`4_000_000`) flush semantics unchanged.

`ExascriptTableData.rows_in_group` ("Rows count in current group in EXASolution", field 8) is set on every batch the engine sends. `create_next_response` writes `table->set_rows_in_group(inp.rowsInGroup())` on both the `MT_NEXT` and the `MT_RESET` reply (`../db/Engine/src/exscript/pluggable/zmqcontainer.cc:415`, recorded in `FINDINGS.md:258-262`). What the value counts follows the input shape. For SET input it is the full group size (`vmciterators.h:147-150`). For SCALAR input it is the current vector-chunk size (`:70-73`). The proto declares the field `required` and comments it "Can be 0 if no group defined" (`zmqcontainer.proto:52-54`), so `0` reports an ungrouped call rather than an absent value. The codec carries the field through to `UdfContext::rows_in_group()`.

`exascript_metadata.input_iter_type` and `output_iter_type` (`zmqcontainer.proto:38-39`, over the `iter_type` enum at `:21`) declare the script's own shapes. `PB_EXACTLY_ONCE` is SCALAR input or RETURNS output, and `PB_MULTIPLE` is SET input or EMITS output. The host context bridge already branches on both axes to drive the run loop per row or per group and to gate `emit` and `next`. It surfaces the same two axes to UDF code through `UdfContext::input_type()` and `output_type()`.
<!-- /DELTA:CHANGED -->

## Scenarios


<!-- DELTA:NEW -->
### Scenario: InputRowSet carries the group row count of the input batch

* *GIVEN* an `ExascriptTableData` the database sent for a set-input group, carrying `rows_in_group`
* *WHEN* `InputRowSet::from_proto` decodes the batch and the host bridge answers `UdfContext::rows_in_group()`
* *THEN* the bridge MUST return that batch's `rows_in_group` value unchanged, and `0` when the database sent none
* *AND* the value MUST stay constant for every row of one group, including across the batch boundaries the group spans
* *AND* for SCALAR input the reported value MUST be the vector-chunk size the engine filled, because the codec passes the field through without interpreting the input shape
* *AND* `EmitBuffer::take_proto` MUST keep writing `rows_in_group = 0` on emitted batches, matching the reference C++ SLC
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: The host context reports the iteration axes the database declared

* *GIVEN* handshake metadata declaring an input iteration axis and an output iteration axis
* *WHEN* a UDF reads `ctx.input_type()` and `ctx.output_type()`
* *THEN* the host context bridge MUST report `Scalar` for `PB_EXACTLY_ONCE` input and `Set` for `PB_MULTIPLE` input, and `Returns` for `PB_EXACTLY_ONCE` output and `Emits` for `PB_MULTIPLE` output
* *AND* one field per axis MUST hold the value, so the accessor and the existing `emit` and `next` gates read the same axis rather than two copies of it
* *AND* the single-call context MUST report the same two axes from the same handshake metadata, because a spec-generation hook reads the declaration of the script it runs in
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: The database reports a non-zero group row count over a live connection

* *GIVEN* a live Exasol database with a registered Rust SLC, the `rows-in-group` fixture registered as a SET script, and a source table of known group cardinality
* *WHEN* a client runs a query that groups that table and calls the script once per group
* *THEN* each emitted row MUST report a group row count equal to the number of rows the UDF iterated with `ctx.next()`
* *AND* that count MUST be non-zero, because the engine fills `rows_in_group` from the group's own cardinality on every batch it sends
<!-- /DELTA:NEW -->
