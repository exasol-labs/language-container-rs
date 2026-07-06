# Feature: rowset-codec

Packs and unpacks UDF row values against the wire's row-major proto type blocks — `EmitBuffer` (output encoding, flush-threshold accounting, the opt-in Arrow batch-emit path, and any promoted fast-path formatter) and `InputRowSet` (decode, including any promoted fast-path parser). Driven by `runtime/dispatch-run-loop`, which owns the scalar/set dispatch loop and calls into this codec to materialise input rows and buffer/flush emitted output; this feature specifies the codec's own packing, flushing, and byte-identity guarantees in isolation from that driving loop.

## Background

The rowset codec (`InputRowSet`/`EmitBuffer`) packs output values by declared column `ExaType` rather than by runtime `Value` variant. The decode path parses TIMESTAMP via `%.f` (0..9 fractional digits, lossless), but the emit path historically hardcoded exactly 6 fractional digits (`%.6f`) — capping `TIMESTAMP(7/8/9)` columns at microseconds. The Exasol engine truncates an emitted timestamp to the output column's declared precision on receipt (`SWIGResultHandler::setTimestamp` parses `YYYY-MM-DD HH24:MI:SS.FF9` and applies `trunc_to_fractional_seconds_precision(value, m_types[col].prec)`, verified in `../db/Engine/src/exscript/pluggable/swigcontainers_int.h:1064-1082` and `zmqcontainer.cc:675`). Therefore emitting MORE fractional digits than the column declares is safe (the engine truncates); emitting FEWER loses precision. This delta makes the emit always carry the full available nanosecond precision (`%.9f`) so the engine's own truncation yields the exact declared precision — the SLC does not truncate client-side and does not need the output column metadata threaded into the encoder. This concerns the **emit/output** path only; it lets UDF-*generated* sub-microsecond values (wall-clock, connect-back data) reach an output column at up to nanosecond precision. It does NOT widen UDF *input*: the engine delivers every input column at microsecond precision (`SWIGTableData::getTimestamp` formats `...FF6`, `swigcontainers_int.h:779-781`), so an input→output round-trip through a UDF is capped at microseconds regardless of this emit format.

This feature also covers an opt-in Arrow batch-emit path behind the `emit-arrow` feature. Because an Arrow `RecordBatch` cannot cross the `.so` boundary (two independently linked static `arrow` copies disagree on `Arc<dyn Array>` vtables / `TypeId`, a hard memory fault), the UDF serialises the batch to Arrow IPC bytes and the host's `HostContextBridge::emit_record_batch_ipc(&[u8])` deserialises them into a host-owned `RecordBatch` before encoding — only `&[u8]` crosses the boundary. `EmitBuffer` gains `push_batch(&RecordBatch, &[ColumnMeta])`, which encodes that host-owned batch's Arrow columns vectorised, column-at-a-time (each column array downcast once, its null buffer read once in bulk — never per cell) into the proto type blocks chosen by the declared output `ExaType`, with no intermediate `Vec<Value>` for the bulk of the data. Because the `MT_EMIT` wire limit `EMIT_BUFFER_LIMIT_BYTES` (`4_000_000`) is a hard cap and a batch's serialised size is unknown at compile time (a batch may exceed it), the encoder computes a cheap cumulative per-row byte cost (fixed-width types by width; variable-width via the Arrow offset buffer; Decimal/Timestamp via the same fixed estimate the row path uses), finds row-granular split points at the 4 MB threshold, and flushes each ≤4 MB zero-copy `RecordBatch::slice` directly — it cannot flush strictly at batch boundaries. The trailing <4 MB remainder is materialised once into the shared `EmitBuffer` so the row-based `emit` path and the single end-of-`run` tail flush stay coherent. The row-based `push`/`emit`/`to_proto` path and all flush semantics are unchanged.

The exact wire-format strings the Exasol engine parses are fixed contracts: `DATE_FORMAT = "%Y-%m-%d"`, `TIMESTAMP_EMIT = "%Y-%m-%d %H:%M:%S%.9f"` (full nanosecond precision, engine-truncated to the declared column precision), and fixed-point decimal via `Decimal`'s `Display`. Any performance optimisation of the formatting/parsing path — whether a hand-rolled fast formatter, a promoted columnar transport spike, or a fast decimal/date parser — must leave those wire bytes and the `EMIT_BUFFER_LIMIT_BYTES` (`4_000_000`) flush semantics unchanged.

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

### Scenario: EmitBuffer encodes an Arrow batch column-at-a-time into proto type blocks

* *GIVEN* an `EmitBuffer` and an `arrow` `RecordBatch` whose columns supply the cell values for an EMITS output whose declared `ColumnMeta` list (the output schema) dictates the target `ExaType` of each column
* *WHEN* `EmitBuffer::push_batch(batch, meta)` is called, gated behind `#[cfg(feature = "emit-arrow")]`
* *THEN* it MUST encode the batch vectorised, column-at-a-time — downcasting each Arrow column array to its concrete type ONCE (not once per cell) and reading its validity/null buffer ONCE in bulk — packing each column's values into the proto type block dictated by the declared column `ExaType`, WITHOUT materialising an intermediate `Vec<Value>` per row for the bulk of the data
* *AND* the resulting proto type blocks MUST preserve the dense, row-major-interleaved layout `to_proto` produces (one cursor per block type advancing across rows, columns of the same `ExaType` interleaved), and a NULL cell (per the Arrow null bitmap) MUST NOT occupy a slot in its type block — only the row-major null bitmap is updated — so the encoding stays byte-identical to the row path for arbitrary EMITS schemas including multiple columns sharing one block type
* *AND* the per-row byte estimate MUST be computed from the SAME accounting the row path uses (fixed-width types by their width, variable-width `Utf8`/`LargeUtf8` from the Arrow offset buffer, `Decimal128`/`Timestamp`/`Date` via the same fixed estimate as `value_byte_cost`, NULL costing 0), so `should_flush` fires at the same threshold the row path would
* *AND* a column whose Arrow `DataType` cannot feed the declared `ExaType` block MUST return `Err(UdfError)` rather than silently emitting a wrong-typed or default value

### Scenario: push_batch splits an oversized batch at row boundaries under the 4 MB cap

* *GIVEN* an `EmitBuffer` with a flusher and an `arrow` `RecordBatch` whose serialised size exceeds `EMIT_BUFFER_LIMIT_BYTES` (`4_000_000`) — a size unknown until the cumulative per-row byte cost is computed
* *WHEN* `EmitBuffer::push_batch(batch, meta)` is called
* *THEN* because the `MT_EMIT` wire limit is a hard cap a single batch can exceed, the encoder MUST NOT flush strictly at the batch boundary; it MUST instead find row-granular split points where the running byte total (including any bytes already buffered from a prior `push`/`push_batch`) reaches the threshold
* *AND* each full ≤4 MB row range MUST be taken as a zero-copy `RecordBatch::slice(offset, len)` (sharing Arrow buffers, no data copy), encoded vectorised into one `ExascriptTableData`, and flushed — so a batch larger than 4 MB produces multiple `MT_EMIT` flushes at correct row boundaries
* *AND* the trailing partial range (strictly below `EMIT_BUFFER_LIMIT_BYTES` by construction) MUST be materialised once into the shared `EmitBuffer` as `Value` rows so a later `emit`/`push_batch` or the end-of-`run` tail flush sends it — this sub-4 MB tail pivot is the only `Value` materialisation on the batch path and it cannot exceed the existing buffer's 4 MB ceiling

### Scenario: push_batch produces proto blocks identical to the row-based push path

* *GIVEN* an `arrow` `RecordBatch` and the equivalent rows expressed as `Vec<Value>`, with one shared declared `ColumnMeta` output schema
* *WHEN* one `EmitBuffer` is filled via `push_batch` and another via row-based `push`, and both are serialised with `to_proto`
* *THEN* the two `ExascriptTableData` results MUST be byte-identical, proving the columnar and row paths converge on the same wire encoding
* *AND* decoding the `push_batch` result via `InputRowSet::from_proto` with the same `meta` MUST reproduce the batch's values

### Scenario: Bridge deserialises emit_batch IPC bytes, carries the output metadata, and flushes on the same threshold

* *GIVEN* a `HostContextBridge` (built with `emit-arrow` enabled) that holds the EMITS output `ColumnMeta` slice — threaded into its constructor by dispatch (the same `meta.output_columns` the flusher serialises with) because the bridge previously held only its input columns — whose `flusher` serialises and clears the `EmitBuffer`
* *WHEN* a UDF calls `ctx.emit_batch(&batch)` during `run` (which serialises to Arrow IPC bytes UDF-side and invokes the bridge's `emit_record_batch_ipc(&[u8])` ABI override)
* *THEN* the bridge's `emit_record_batch_ipc` override MUST deserialise the IPC bytes into a host-owned `RecordBatch`, call `self.emit_buf.push_batch(batch, output_meta)` against that output column metadata, then check `should_flush()` and invoke the `flusher` when the byte estimate has reached `EMIT_BUFFER_LIMIT_BYTES` — the identical buffer-then-flush control flow as the row-based `emit`
* *AND* the bridge MUST surface a deserialise error or a `push_batch` error as the method's `Err`, so a malformed batch fails the UDF run rather than being silently dropped

### Scenario: emit and emit_batch share one buffer and one tail flush

* *GIVEN* a `HostContextBridge` (built with `emit-arrow` enabled) and a UDF that interleaves `ctx.emit(&[Value])` and `ctx.emit_batch(&batch)` calls within one `run`
* *WHEN* the UDF returns from `run`
* *THEN* rows from both emit styles MUST accumulate into the same `EmitBuffer`, so the two styles are interchangeable and produce one coherent output stream
* *AND* the dispatch loop's existing tail flush MUST send any rows still buffered from either style, so no emitted row is lost even when the threshold was never reached

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
