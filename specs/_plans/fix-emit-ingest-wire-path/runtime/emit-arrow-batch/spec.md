# Feature: emit-arrow-batch

Encodes an Arrow `RecordBatch` directly into the wire's row-major proto type blocks via `EmitBuffer::push_batch`, gated behind the opt-in `emit-arrow` feature, so a UDF can emit columnar Arrow data without a per-row `Value` conversion while sharing the same `EMIT_BUFFER_LIMIT_BYTES` flush threshold and byte-identical wire encoding as the row-based `push`/`emit` path specified in `runtime/rowset-codec`.

## Background

Because an Arrow `RecordBatch` cannot cross the `.so` boundary (two independently linked static `arrow` copies disagree on `Arc<dyn Array>` vtables / `TypeId`, a hard memory fault), the UDF serialises the batch to Arrow IPC bytes and the host's `HostContextBridge::emit_record_batch_ipc(&[u8])` deserialises them into a host-owned `RecordBatch` before encoding — only `&[u8]` crosses the boundary. `EmitBuffer` gains `push_batch(&RecordBatch, &[ColumnMeta])`, which encodes that host-owned batch's Arrow columns vectorised, column-at-a-time (each column array downcast once, its null buffer read once in bulk — never per cell) into the proto type blocks chosen by the declared output `ExaType`, with no intermediate `Vec<Value>` for the bulk of the data. Because the `MT_EMIT` wire limit `EMIT_BUFFER_LIMIT_BYTES` (`4_000_000`) is a hard cap and a batch's serialised size is unknown at compile time (a batch may exceed it), the encoder computes a cheap cumulative per-row byte cost (fixed-width types by width; variable-width via the Arrow offset buffer; Decimal/Timestamp via the same fixed estimate the row path uses), finds row-granular split points at the 4 MB threshold, and flushes each ≤4 MB zero-copy `RecordBatch::slice` directly — it cannot flush strictly at batch boundaries. The trailing <4 MB remainder is materialised once into the shared `EmitBuffer` so the row-based `emit` path and the single end-of-`run` tail flush stay coherent. The row-based `push`/`emit`/`to_proto` path and all flush semantics — specified in `runtime/rowset-codec` — are unchanged.

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: push_batch produces proto blocks identical to the row-based push path

* *GIVEN* an `arrow` `RecordBatch` and the equivalent rows expressed as `Vec<Value>`, with one shared declared `ColumnMeta` output schema
* *WHEN* one `EmitBuffer` is filled via `push_batch` and another via row-based `push`, and both are serialised with `to_proto`
* *THEN* the two `ExascriptTableData` results MUST be byte-identical, proving the columnar and row paths converge on the same wire encoding
* *AND* decoding the `push_batch` result via `InputRowSet::from_proto` with the same `meta` MUST reproduce the batch's values
* *AND* when the input axis is `ExactlyOnce`, `encode_slice` MUST fill the `row_number` array with the current input row's number, one entry per encoded row, so a batch emitted from a scalar UDF resolves pass-through output columns exactly as the row path does
* *AND* when the input axis is `Multiple` the `row_number` array MUST stay empty, matching the row path
<!-- /DELTA:CHANGED -->
</content>
