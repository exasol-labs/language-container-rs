# Feature: dispatch-run-loop

The runtime drives dispatch via the pure protocol state machine after a UDF `.so` has been loaded. This change makes `ctx.emit` bound its memory: the bridge accumulates rows in an `EmitBuffer` and flushes an `MT_EMIT` to the wire when a running byte estimate reaches `EMIT_BUFFER_LIMIT_BYTES` (`4_000_000`), then flushes any tail at end of `run`.

## Background

Emitted rows are buffered, not sent one frame per row. The bridge holds a wire-flusher closure (threaded in by the dispatch loop, mirroring the existing `conn_requester`) that serializes the buffer, sends `MT_EMIT`, awaits the ack, and clears the buffer. The dispatch loop is blocked awaiting the UDF's `run` return during emit, so the ZMQ socket is idle and a mid-run flush is safe.

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Set/EMITS dispatch emits multiple rows across batches

* *GIVEN* a loaded set UDF and a `HostContextBridge` with a wire flusher closure that serializes the `EmitBuffer` to an `ExascriptTableData`, sends `MT_EMIT`, awaits the emit ack, and clears the buffer
* *WHEN* the UDF iterates all input rows and emits a filtered subset
* *THEN* the bridge MUST accumulate emitted rows in the `EmitBuffer` and MUST trigger a mid-run `MT_EMIT` flush as soon as the buffer's running byte estimate reaches the `EMIT_BUFFER_LIMIT_BYTES` threshold of `4_000_000` bytes, rather than sending one frame per row or buffering an unbounded batch
* *AND* after the UDF's `run` method returns, the dispatch loop MUST flush any remaining buffered rows in a final `MT_EMIT` even when the byte estimate is below the threshold, so no emitted row is lost
* *AND* the total emitted row count across all flushes MUST equal the number of `emit` calls the UDF made
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: A single emitted row larger than the flush threshold is sent on its own

* *GIVEN* a loaded set UDF whose single emitted row carries a value whose serialized size alone exceeds `EMIT_BUFFER_LIMIT_BYTES` (`4_000_000` bytes)
* *WHEN* the UDF calls `emit` once with that oversized row
* *THEN* the bridge MUST push the whole row into the `EmitBuffer` as one unit and MUST NOT split a single row across `MT_EMIT` frames, because the wire protocol packs rows atomically
* *AND* the bridge MUST then observe that the buffer's byte estimate crosses the threshold and flush the single-row buffer in one `MT_EMIT`, accepting that the frame exceeds the nominal 4,000,000-byte target rather than dropping or truncating the row
* *AND* the only hard ceiling that remains MUST be the protocol's 2 GB per-value limit, which the runtime does not attempt to circumvent
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: EmitBuffer tracks a running byte estimate and reports when to flush

* *GIVEN* a fresh `EmitBuffer`
* *WHEN* rows are appended via `push`
* *THEN* `push` MUST increase a `byte_estimate` field by an approximation of the wire size of the pushed values (summing per-value byte costs), and `should_flush` MUST return true exactly when `byte_estimate` is greater than or equal to `EMIT_BUFFER_LIMIT_BYTES` (`4_000_000`)
* *AND* `clear` MUST reset both the row vector and the `byte_estimate` to zero so a flushed buffer starts a fresh accounting cycle
* *AND* the byte estimate MUST be a monotonic non-negative running total computed without re-serializing the whole buffer on every `push`, so emit cost stays linear in the number of rows
<!-- /DELTA:NEW -->
