# Tasks: fix-emit-ingest-wire-path

## PR Lifecycle
- [x] resolved
- [x] implemented
- [x] version-bumped
- [ ] tested-green
- [ ] recorded
- [ ] pr-ready

## Phase 2: Implementation (Group A: emit/ingest codec and emit ownership)
- [x] 1.1 Retain the MT_NEXT batch's row_number array in InputRowSet and expose the current row's number.
- [x] 1.2 Stamp the current input row's number onto each row push_output_row buffers when the input axis is ExactlyOnce, serialise the array in to_proto, and keep it empty for Multiple and for a batch that supplies fewer entries than rows. [expert]
- [x] 1.3 Fill row_number in encode_slice under the same rule, so the Arrow batch path keeps byte parity with the row path.
- [x] 1.4 Add IT scenario scalar_emits_passthrough_column_resolves_source_row: SELECT k, emit_k(k) FROM it_rust.emit_k_src asserting each pass-through value equals its source input, then repeat over ORDINAL_100K so the assertion crosses a scalar MT_NEXT batch boundary.
- [x] 1.5 Replace NUMERIC_COST_BASE with the exact rendered-decimal length from checked_ilog10, and cost Arrow Decimal128(p, s) as p + 2.
- [x] 1.6 Change to_proto to &mut self, consume the buffered rows, and move strings through value_into_block_string.
- [x] 1.7 Add emit_owned(Vec<Value>) to UdfContext with a forwarding default, override it in HostContextBridge to move the row, bump EXA_UDF_ABI_VERSION 7 → 8, and switch the emit-k fixture to emit_owned so the new vtable slot is exercised end to end. [expert]
- [x] 1.8 Restructure InputRowSet to keep the proto typed arrays as storage: take the table by value, drain data_string with into_iter, materialise the current row into one reusable scratch Vec<Value>, and replace row(idx) with forward-only seek_row(idx). [expert]
- [x] 1.9 Hoist the EmitBuffer, the Protocol cell, the emit flusher and the batch fetcher from run_group to session scope in run_udf, add the capacity-retaining group reset that leaves flush_count untouched, and reserve from rows_in_group under a constant row cap. [expert]
- [x] 1.10 Add crates/exa-udf-runtime/tests/ingest_alloc.rs with a counting global allocator, asserting the allocation count of from_proto plus a full row walk does not grow with the batch's row count.

## Phase 2: Implementation (Group B: ZMQ transport frames)
- [x] 2.1 Remove MAX_TOTAL_WAIT, its elapsed-time branch and the timeout ProtocolError, and extract the retry loop into a socket-free helper so it is unit-testable.
- [x] 2.2 Send the encoded frame as zmq::Message::from(Vec<u8>), re-encoding the request on each EAGAIN retry attempt.
- [x] 2.3 Decode the response from recv_msg's byte slice instead of recv_bytes.
- [x] 2.4 Add crates/exa-zmq-protocol/src/transport_tests.rs (declared as the last item of transport.rs) covering the uncapped retry, and extend crates/exa-zmq-protocol/tests/transport.rs for the message-based send and slice-based decode.

## Phase 2: Implementation (Group C: connect-back streaming and diagnostics)
- [x] 3.1 Drive batch iteration inside the single block_on async block, invoke the row callback from there, and drop fetch_all. [expert]
- [x] 3.2 Replace cb_log with tracing::debug! carrying the SQL as a structured field, and delete the file-append path.
- [x] 3.3 Extend crates/exa-udf-runtime/tests/debug_level.rs to assert the connect-back events are level-gated and that no diagnostic file is created.

## Phase 2: Implementation (Group D: docs, build profile and benchmark)
- [x] 4.1 Correct the integer mapping in docs/writing-a-udf.md: INT/INTEGER is DECIMAL(18,0) and arrives as Value::Int64; only BIGINT (DECIMAL(36,0)) arrives as Value::Numeric. Fix the annotation guidance at line 170 and the Value reference note at line 217.
- [x] 4.2 Add a performance section to docs/writing-a-udf.md: prefer DECIMAL(18,0)/INTEGER over BIGINT below 19 digits, prefer DOUBLE over scaled DECIMAL where acceptable, prefer VARCHAR over CHAR(n), never emit an empty string (the engine stores it as NULL), and input TIMESTAMP always arrives at microsecond precision.
- [x] 4.3 Set lto = "fat" and codegen-units = 1 in the workspace [profile.release] and in the scaffold template in crates/cargo-exasol-udf/src/new.rs, keeping panic = "unwind".
- [x] 4.4 Add a native benchmark shape to benches/emit-bench using only DECIMAL(18,0) and DOUBLE columns, and correct the mixed shape's description and bytes_per_row estimate, which both assume id BIGINT travels as a native integer.

## Phase 3: Verification
- [ ] 5.1 Run automated checklist (build, test, clippy, fmt)
- [ ] 5.2 Scenario coverage audit
- [ ] 5.3 Manual verification steps

## Phase 4: Review Fixes
- [x] 6.1 In crates/exa-udf-runtime/src/dispatch.rs, introduce a private struct `SessionWire<'s>` bundling `transport: &'s ZmqTransport`, `proto_cell: &'s RefCell<&'s mut Protocol>`, and `exit: &'s Cell<Option<GroupExit>>`. Pass it as a single argument to `run_group`, `emit_flusher`, `batch_fetcher`, and `tail_flush`, replacing the three individual parameters in each signature.
- [x] 6.2 In crates/exa-udf-runtime/src/rowset_tests.rs, add a test `reserve_rows_caps_at_max_reserve_rows` that constructs an `EmitBuffer`, calls `reserve_rows(100_000_000)`, and asserts `emit.rows.capacity() <= EmitBuffer::MAX_RESERVE_ROWS`.
- [x] 6.3 In benches/emit-bench-udf/src/lib.rs, append one character to the `LABEL` literal (e.g. `"01234567890123456789012345678901234567890123456789"`, 50 characters) so it matches the doc comment and the driver's `bytes_per_row` estimate.
