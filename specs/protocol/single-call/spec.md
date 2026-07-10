# Feature: single-call

Dispatches the single-call `SC_FN_*` path — decoding `MT_CALL` into a host event, serializing the host's result back as `MT_RETURN`, and handling the undefined-hook and DB-acknowledgement cases.

## Background

v2 extends the protocol with the single-call path (`MT_CALL`, `MT_RETURN`, `MT_UNDEFINED_CALL`) carrying a `SingleCallFunctionId`, selected when `MT_META` carries `single_call_function_id != SC_FN_NIL`. In single-call mode the DB acknowledges the container's `MT_RETURN` result by echoing `MT_RETURN`; the state machine surfaces this as `HostEvent::SingleCallAck` so the dispatch loop can close the run with `MT_DONE`. The protocol MUST NOT advance to the close sequence on `MT_RETURN` alone — the session ends only on a subsequent `MT_CLEANUP`. In non-single-call mode, `MT_RETURN` in the run phase remains a protocol error.

## Scenarios

### Scenario: Single-call request surfaces a SingleCall host event

* *GIVEN* a `Protocol` driven past the handshake where `MT_META` carried `single_call_function_id != SC_FN_NIL`
* *WHEN* the database sends an `MT_CALL` response carrying a `single_call_function_id` and its payload
* *THEN* the state machine MUST emit a `HostEvent::SingleCall` carrying the decoded `SingleCallFn` and the call arguments
* *AND* it MUST NOT emit any scalar/set run events (`HostEvent::Next` or `HostEvent::Run`) for that exchange
* *AND* the state machine MUST remain pure, performing no socket I/O

### Scenario: Single-call return is serialized to MT_RETURN

* *GIVEN* a `Protocol` in single-call mode that has emitted a `HostEvent::SingleCall`
* *WHEN* the host supplies a `HostAction::SingleCallReturn` carrying a result payload string
* *THEN* the next `next_request` MUST emit an `MT_RETURN` request whose body carries the result payload
* *AND* when the DB echoes `MT_RETURN` as the acknowledgement, the state machine MUST emit `HostEvent::SingleCallAck` so the dispatch loop can close the run with `MT_DONE`
* *AND* the protocol MUST NOT advance to the close sequence on `MT_RETURN` alone — the session ends only on a subsequent `MT_CLEANUP`

### Scenario: Unimplemented single-call hook is serialized to MT_UNDEFINED_CALL

* *GIVEN* a `Protocol` that has emitted a `HostEvent::SingleCall`
* *WHEN* the host supplies a `HostAction::UndefinedCall`
* *THEN* the next `next_request` MUST emit an `MT_UNDEFINED_CALL` request
* *AND* the protocol MUST then advance to the close sequence

### Scenario: MT_RETURN DB acknowledgement in single-call mode surfaces SingleCallAck

* *GIVEN* a `Protocol` in single-call mode where `single_call_mode = true`
* *WHEN* the DB sends `MT_RETURN` in the Run phase (acknowledging the container's `MT_RETURN` result)
* *THEN* the state machine MUST emit `HostEvent::SingleCallAck` and MUST NOT treat this as a protocol error
* *AND* in a non-single-call protocol, an `MT_RETURN` received in the Run phase MUST still be surfaced as a `ProtocolError::UnexpectedMessage`
