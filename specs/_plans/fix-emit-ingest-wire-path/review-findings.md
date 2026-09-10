# Code Review Findings: fix-emit-ingest-wire-path

## Summary
- Files reviewed: 25
- Total findings: 3 (standard: 3, expert: 0)

## Standard fixes

### crates/exa-udf-runtime/src/dispatch.rs

#### [TOO_MANY_ARGUMENTS] run_group takes 6 parameters after emit_buf hoisting
- Location: line 70
- Issue: `run_group` takes `transport`, `proto_cell`, `exit`, `emit_buf`, `udf`, `meta` (6 parameters). The first three are the session's wire plumbing and travel together at every call site.
- Fix: In `crates/exa-udf-runtime/src/dispatch.rs`, introduce a private struct `SessionWire<'s>` bundling `transport: &'s ZmqTransport`, `proto_cell: &'s RefCell<&'s mut Protocol>`, and `exit: &'s Cell<Option<GroupExit>>`. Pass it as a single argument to `run_group`, `emit_flusher`, `batch_fetcher`, and `tail_flush`, replacing the three individual parameters in each signature.

### crates/exa-udf-runtime/src/rowset_tests.rs

#### [MISSING_BOUNDARY_TEST] reserve_rows cap at MAX_RESERVE_ROWS is untested
- Location: (absent test)
- Issue: `EmitBuffer::reserve_rows` caps the pre-allocation at `MAX_RESERVE_ROWS` (65,536) to prevent an unbounded group from reserving gigabytes. No test asserts this cap: passing a `rows_in_group` value above the cap and asserting the resulting capacity stays at or below the cap. The cap is the sole guard against the OOM risk the plan's Consequences section calls out.
- Fix: In `crates/exa-udf-runtime/src/rowset_tests.rs`, add a test `reserve_rows_caps_at_max_reserve_rows` that constructs an `EmitBuffer`, calls `reserve_rows(100_000_000)`, and asserts `emit.rows.capacity() <= EmitBuffer::MAX_RESERVE_ROWS`.

### benches/emit-bench-udf/src/lib.rs

#### [OUTDATED_COMMENT] LABEL doc comment says 50-char but the literal is 49 characters
- Location: line 46
- Issue: The doc comment on `LABEL` reads "50-char payload, comfortably inside VARCHAR(100)" but the string literal `"0123456789012345678901234567890123456789012345678"` is 49 characters, not 50. The benchmark driver's `bytes_per_row` estimate for the `mixed` shape uses 50 for the label contribution, so the estimate is also off by one byte per row.
- Fix: In `benches/emit-bench-udf/src/lib.rs`, append one character to the `LABEL` literal (e.g. `"01234567890123456789012345678901234567890123456789"`, 50 characters) so it matches the doc comment and the driver's `bytes_per_row` estimate.

## Expert fixes
[none]
