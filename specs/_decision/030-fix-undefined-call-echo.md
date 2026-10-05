# Decisions: fix-undefined-call-echo

## ADR: Accept the DB's MT_UNDEFINED_CALL echo

**ID:** undefined-call-echo-ack
**Plan:** fix-undefined-call-echo
**Status:** Accepted

### Context

In single-call mode the DB acknowledges the container's reply by echoing the message it sent, and ends the session after the container sends `MT_DONE`. A dynamic-EMITS script called without an EMITS clause reaches `MT_UNDEFINED_CALL`, because the `#[exasol_udf]` macro leaves `SC_FN_DEFAULT_OUTPUT_COLUMNS` unimplemented.

### Decision

An `MT_UNDEFINED_CALL` in the Run phase surfaces as `HostEvent::UndefinedCallAck` in single-call mode and is a protocol error outside it, as `MT_RETURN` is. The dispatcher requires the ack to echo the message it sent. An `MT_RETURN` answering an `MT_UNDEFINED_CALL` is a hard error.

### Options Considered

| Option | Verdict |
|--------|---------|
| Own `UndefinedCallAck` event, ack must echo the reply | ✓ Chosen |
| Reuse `SingleCallAck` for both echoes | ✗ Accepts a desynchronised exchange |
| Publish the annotated `emits(...)` schema as default output columns | ✗ Separate capability, and unannotated UDFs still need the undefined path |

### Consequences

A dynamic-EMITS script called without an EMITS clause fails with the DB's error text, and any unimplemented single-call hook ends over the normal `MT_DONE` / `MT_CLEANUP` / `MT_FINISHED` sequence.
