# Feature: emit-arrow-batch

Encodes an Arrow `RecordBatch` directly into the wire's row-major proto type blocks via `EmitBuffer::push_batch`, gated behind the opt-in `emit-arrow` feature, so a UDF can emit columnar Arrow data without a per-row `Value` conversion while sharing the same buffer, `EMIT_BUFFER_LIMIT_BYTES` flush threshold and byte-identical wire encoding as the row-based `push`/`emit` path specified in `runtime/rowset-codec`.

## Background

Because an Arrow `RecordBatch` cannot cross the `.so` boundary (two independently linked static `arrow` copies disagree on `Arc<dyn Array>` vtables / `TypeId`, a hard memory fault), the UDF serialises the batch to Arrow IPC bytes and the host's `HostContextBridge::emit_record_batch_ipc(&[u8])` deserialises them into a host-owned `RecordBatch` before encoding — only `&[u8]` crosses the boundary. `EmitBuffer` gains `push_batch(&RecordBatch, &[ColumnInfo], row_number)`, which downcasts each Arrow column array once and reads its null buffer once in bulk — never per cell — and appends the batch's rows into the same proto type blocks the row path fills, choosing each block by the declared output `ExaType`, with no intermediate `Vec<Value>`. Because the `MT_EMIT` wire limit `EMIT_BUFFER_LIMIT_BYTES` (`4_000_000`) is a hard cap and a batch's serialised size is unknown at compile time (a batch may exceed it), the encoder flushes at row boundaries as the running estimate crosses the threshold — it cannot flush strictly at batch boundaries. Anything still buffered stays for a later `emit`/`push_batch` or the single end-of-`run` tail flush. The row-based `emit` path and all flush semantics are specified in `runtime/rowset-codec`; this feature shares them rather than duplicating them.

## Scenarios

### Scenario: EmitBuffer encodes an Arrow batch column-at-a-time into proto type blocks

* *GIVEN* an `EmitBuffer` and an `arrow` `RecordBatch` whose columns supply the cell values for an EMITS output whose declared `ColumnInfo` list (the output schema) dictates the target `ExaType` of each column
* *WHEN* `EmitBuffer::push_batch(batch, meta, row_number)` is called, gated behind `#[cfg(feature = "emit-arrow")]`
* *THEN* it MUST downcast each Arrow column array to its concrete type ONCE (not once per cell) and read its validity/null buffer ONCE in bulk, packing each column's values into the proto type block dictated by the declared column `ExaType`, WITHOUT materialising an intermediate `Vec<Value>` for any part of the batch
* *AND* the resulting proto type blocks MUST preserve the dense, row-major-interleaved layout the row path produces (one cursor per block type advancing across rows, columns of the same `ExaType` interleaved), and a NULL cell (per the Arrow null bitmap) MUST NOT occupy a slot in its type block — only the row-major null bitmap is updated — so the encoding stays byte-identical to the row path for arbitrary EMITS schemas including multiple columns sharing one block type
* *AND* the byte estimate MUST be computed from the SAME accounting the row path uses (fixed-width types by their width, variable-width `Utf8`/`LargeUtf8` from the Arrow offset buffer, `Decimal128`/`Timestamp`/`Date` via the same fixed estimate as `value_byte_cost`, NULL costing 0), so `should_flush` fires at the same threshold the row path would
* *AND* a batch whose whole-batch estimate — computed in O(columns) from the null counts and offset-buffer spans, and never an under-count — cannot reach the threshold MUST be appended without computing a per-row cost vector
* *AND* a column whose Arrow `DataType` cannot feed the declared `ExaType` block MUST return `Err(UdfError)` rather than silently emitting a wrong-typed or default value

### Scenario: push_batch splits an oversized batch at row boundaries under the 4 MB cap

* *GIVEN* an `EmitBuffer` with a flusher and an `arrow` `RecordBatch` whose serialised size exceeds `EMIT_BUFFER_LIMIT_BYTES` (`4_000_000`) — a size unknown until the cumulative per-row byte cost is computed
* *WHEN* `EmitBuffer::push_batch(batch, meta, row_number)` is called
* *THEN* because the `MT_EMIT` wire limit is a hard cap a single batch can exceed, the encoder MUST NOT flush strictly at the batch boundary; it MUST instead flush at the row boundary where the running byte total (including any bytes already buffered from a prior `push`/`push_batch`) reaches the threshold — so a batch larger than 4 MB produces multiple `MT_EMIT` flushes at correct row boundaries
* *AND* the rows left over below the threshold MUST stay in the shared `EmitBuffer` so a later `emit`/`push_batch` or the end-of-`run` tail flush sends them
* *AND* every row of the batch, in the flushed slices and in the buffered tail alike, MUST carry the row number of the input row being processed, since the whole batch was emitted from that one row (`runtime/rowset-codec`)

### Scenario: push_batch produces proto blocks identical to the row-based push path

* *GIVEN* an `arrow` `RecordBatch` and the equivalent rows expressed as `Vec<Value>`, with one shared declared `ColumnInfo` output schema
* *WHEN* one `EmitBuffer` is filled via `push_batch` and another via row-based `push`, and both are serialised with `take_proto`
* *THEN* the two `ExascriptTableData` results MUST be byte-identical, proving the columnar and row paths converge on the same wire encoding
* *AND* decoding the `push_batch` result via `InputRowSet::from_proto` with the same `meta` MUST reproduce the batch's values

### Scenario: Bridge deserialises emit_batch IPC bytes, carries the output metadata, and flushes on the same threshold

* *GIVEN* a `HostContextBridge` (built with `emit-arrow` enabled) that holds the EMITS output `ColumnInfo` slice — threaded into its constructor by dispatch (the same `meta.output_columns` the flusher serialises with) because the bridge previously held only its input columns — whose `flusher` serialises and clears the `EmitBuffer`
* *WHEN* a UDF calls `ctx.emit_batch(&batch)` during `run` (which serialises to Arrow IPC bytes UDF-side and invokes the bridge's `emit_record_batch_ipc(&[u8])` ABI override)
* *THEN* the bridge's `emit_record_batch_ipc` override MUST deserialise the IPC bytes into a host-owned `RecordBatch` and call `self.emit_buf.push_batch(batch, output_meta)` against that output column metadata, which invokes the `flusher` whenever the byte estimate reaches `EMIT_BUFFER_LIMIT_BYTES` — the identical buffer-then-flush contract as the row-based `emit`
* *AND* the bridge MUST surface a deserialise error or a `push_batch` error as the method's `Err`, so a malformed batch fails the UDF run rather than being silently dropped

### Scenario: emit and emit_batch share one buffer and one tail flush

* *GIVEN* a `HostContextBridge` (built with `emit-arrow` enabled) and a UDF that interleaves `ctx.emit(vec![Value])` and `ctx.emit_batch(&batch)` calls within one `run`
* *WHEN* the UDF returns from `run`
* *THEN* rows from both emit styles MUST accumulate into the same `EmitBuffer` in call order, so the two styles are interchangeable and produce one coherent output stream
* *AND* neither style may flush what the other has buffered while the byte estimate is below the threshold
* *AND* the dispatch loop's existing tail flush MUST send any rows still buffered from either style, so no emitted row is lost even when the threshold was never reached
