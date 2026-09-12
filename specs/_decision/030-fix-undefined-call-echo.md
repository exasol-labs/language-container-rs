# Decisions: fix-undefined-call-echo

## ADR: Accept the DB's MT_UNDEFINED_CALL echo

**ID:** undefined-call-echo-ack
**Plan:** fix-undefined-call-echo
**Status:** Accepted

### Context

In single-call mode the DB acknowledges the container's reply by echoing the message it sent, and ends the session only after the container closes the run with `MT_DONE`. The state machine had an arm for the `MT_RETURN` echo but none for `MT_UNDEFINED_CALL`, so the echo fell through to `ProtocolError::UnexpectedMessage` and the container closed the wire with `F-UDF-CL-RUST-9001`. The reference C++ SLC requires the echo and continues with `MT_DONE`.

Every EMITS script with dynamic output columns (`EMITS (...)`) called without an EMITS clause reaches this path: the DB asks for `SC_FN_DEFAULT_OUTPUT_COLUMNS`, which the `#[exasol_udf]` macro leaves unimplemented. The user saw a container protocol error instead of the DB's own diagnostic.

### Decision

An `MT_UNDEFINED_CALL` in the Run phase surfaces as `HostEvent::UndefinedCallAck` in single-call mode, and stays a protocol error outside it, as `MT_RETURN` already does. The dispatcher requires the ack to echo the message it sent: an `MT_RETURN` answering an `MT_UNDEFINED_CALL` remains a hard error.

### Options Considered

| Option | Verdict |
|--------|---------|
| Own `UndefinedCallAck` event, ack must echo the reply | ✓ Chosen — parity with the reference, which rejects a mismatched reply type |
| Reuse `SingleCallAck` for both echoes | ✗ Rejected — accepts a desynchronised exchange, the class of bug this one was |
| Publish the annotated `emits(...)` schema as default output columns | ✗ Rejected — a separate capability; unannotated UDFs would still need the undefined path |

### Consequences

A dynamic-EMITS script called without an EMITS clause fails with the DB's error text, and the session ends over the normal `MT_DONE` / `MT_CLEANUP` / `MT_FINISHED` sequence; the same holds for any single-call hook a UDF leaves unimplemented. The scalar/set data path is untouched.
