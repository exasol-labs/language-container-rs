# Plan Review Findings: fix-emit-ingest-wire-path (round 1)

## Summary
- Axes checked: 6/6
- Total findings: 5 (Blockers: 0, Advisory: 5)
- Intent Fidelity blockers: 0

## Intent Fidelity
[no objection -- axis checked: all 11 issue #95 findings (F1-F11) mapped to tasks and spec deltas; no user-requested scope dropped or substituted; non-goals (wire byte format, flush threshold, columnar transport, Arrow-free boundary) match the issue's stated boundaries]

## Feasibility

#### [UNSTATED_ASSUMPTION] ADVISORY
- Location: plan.md task 3.1; connect-back-query delta, CHANGED scenario "query_for_each streams the result set one batch at a time"
- Issue: The spec says the host "MUST drive the whole fetch inside one `block_on`... awaiting each `RecordBatch` in turn" and "MUST NOT call `fetch_all`." The current code comment (connect_back.rs:72-78) says it avoids `ResultSet::into_iterator()` / `next_batch()` because those call `Handle::try_current()` then `handle.block_on(...)`, deadlocking the `current_thread` runtime. The plan names no specific `exarrow-rs` async per-batch method it will use instead and does not document verifying one exists. If the only non-`fetch_all` APIs are the sync `into_iterator`/`next_batch` wrappers, the streaming redesign deadlocks.
- Fix: Add a sentence to task 3.1 naming the `exarrow-rs` async method for per-batch iteration (for example `ResultSet::next().await` or `Stream::next()`) and documenting that it was verified to exist at the pinned version, or note in the decision log that the [expert] implementer must verify the API exists and fall back to a documented alternative if it does not.

#### [EFFORT_MISESTIMATION] ADVISORY
- Location: plan.md task 1.9
- Issue: The task reads "Hoist the `EmitBuffer`, the shared `Protocol` cell, the emit flusher and the batch fetcher from `run_group` to session scope in `run_udf`." The current `run_group` (dispatch.rs:76-122) manages a delicate borrow discipline: the bridge borrows `emit_buf` inside a block scope, releases it for the tail flush, and the `RefCell<&mut Protocol>` is scoped to one group. Hoisting the cell and closures to `run_udf` scope restructures all lifetime relationships between the bridge, the closures, and the protocol across the outer loop (lines 31-50 of `run_udf`), where `proto` is also used directly for `run_request`, `done_request`, and `finished_reply`. The task compresses this into one line.
- Fix: Add a sub-bullet to task 1.9 stating that the `RefCell<&mut Protocol>` hoist requires the outer-loop `run_request`/`done_request`/`finished_reply` calls to borrow and release the cell without overlapping the closures' borrows, so the implementer knows the borrow-scope restructuring is part of the task.

## Requirement Quality

#### [COMPLETENESS_GAP] ADVISORY
- Location: emit-arrow-batch delta, CHANGED scenario "push_batch produces proto blocks identical to the row-based push path," clause "when the input axis is `ExactlyOnce`, `encode_slice` MUST fill the `row_number` array with the current input row's number"
- Issue: `encode_slice` currently receives a `RecordBatch` slice and `&[ColumnMeta]`. The current input row's number is bridge state (from `InputRowSet`), not available in `encode_slice`'s parameter list. The rowset-codec delta specifies how the row number flows for the row path (bridge stamps via `push_output_row`), but no delta specifies how the current input row's number reaches `encode_slice` or `push_batch` on the batch path. The implementer must invent the threading mechanism (new parameter, EmitBuffer field, or bridge setter) without spec guidance.
- Fix: Add a clause to the emit-arrow-batch CHANGED scenario (or to the rowset-codec `row_number` scenario) stating that the bridge provides the current input row number to `push_batch` or that `EmitBuffer` carries it as state set by the bridge before each `push_batch` call.

#### [COMPLETENESS_GAP] ADVISORY
- Location: connect-back-query delta, REMOVED scenario "RuntimeExaConnection streams query results as Value rows"; decision-log.md entry [10]
- Issue: The REMOVED scenario's explanation says "its unique clauses (no `query_arrow`, no `RecordBatch` across the `.so` boundary, `query` collecting the streaming path) are folded into that scenario." Decision [10] repeats this claim. The CHANGED scenario carries the `RecordBatch` boundary clause and the `query` collection clause, but does NOT carry "the host MUST NOT implement or expose `query_arrow`." The claim of folding is inaccurate for that clause. The prohibition is a defensive negative requirement; its protective intent survives via the retained "`RecordBatch` MUST NOT cross the `.so` boundary" clause and the `sdk/udf-abi` vtable spec, but the explicit `query_arrow` surface-level guard is dropped.
- Fix: Either add "the host MUST NOT expose a `query_arrow` method that returns `RecordBatch` across the `.so` boundary" to the CHANGED scenario's clause list, or correct the REMOVED scenario's explanation and decision [10] to say "its unique clauses except the `query_arrow` prohibition are folded; the `query_arrow` surface-level guard is subsumed by the retained `RecordBatch` boundary clause."

## Task Breakdown
[no objection -- axis checked: every spec delta traces to at least one task; every task references a finding (F1-F11) and a spec delta or documented decision (decision 9 for the workspace profile and docs); Group A's size is justified by the shared `rowset.rs` rewrite; Groups A-D share no source files or spec deltas; no parallelization group violates the coherence rule]

## Design Depth
[no objection -- axis checked: plan.md Design Diagnostic answers the quick diagnostic for both new interfaces (`emit_owned` and `InputRowSet`'s narrowed row surface); the `emit_owned` leak is confined by a forwarding default and documented; `InputRowSet` narrowing deepens the module by hiding the storage decision; the session-scoped buffer pattern reduces per-group allocation without widening any interface; no tactical shortcut is taken without rationale]

## Prose Quality

#### [PROSE_BLOAT] ADVISORY
- Location: rowset-codec delta, DELTA:NEW background paragraph (line 3 of the delta block); handshake delta, DELTA:CHANGED background (lines 2 and 3)
- Issue: Three sentences in new/changed background text exceed the 25-word descriptive cap. (1) rowset-codec: "The engine reads `row_number(row)` with no bounds check to resolve every output column the script does not emit (`zmqcontainer.cc:696-707`), and fills the array itself on every `MT_NEXT` from a monotonic per-run counter (`zmqcontainer.cc:567`)." (42 words, joined by `and`). (2) handshake: "A transient ZMQ `EAGAIN` on `send`/`recv` MUST be retried for as long as it continues, with no wall-clock cap, matching the reference `libexaudflib` client and leaving session termination to the database watchdog." (33 words). (3) handshake: "The transport hands each outbound frame to libzmq as an owned `zmq::Message` and decodes each inbound frame from the received message's slice, so one request-reply exchange costs no user-space copy of the frame on either side." (37 words).
- Fix: Split each into two sentences. For example, (1) becomes "The engine reads `row_number(row)` with no bounds check to resolve every output column the script does not emit (`zmqcontainer.cc:696-707`). The engine fills the array itself on every `MT_NEXT` from a monotonic per-run counter (`zmqcontainer.cc:567`)."
