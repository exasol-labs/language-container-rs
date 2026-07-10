# Feature: run-dispatch

Drives the scalar and set/EMITS run loop from `MT_NEXT`/`MT_EMIT` through `MT_DONE`, and handles the ping-pong, reset, try-again, close, and error-close events that punctuate a run.

## Background

Past the handshake, the `Protocol` state machine MUST remain pure — it consumes decoded `ExascriptResponse` values and produces `ExascriptRequest` values and `HostEvent`s without performing any socket I/O, so it can be unit-tested with fixtures. `iter_type` (resolved during the handshake) selects between a single-batch scalar run (`ExactlyOnce`) and a multi-batch set/EMITS run (`Multiple`); both share the same `NEXT` → `InputRows` → `Emit` → `Done` shape.

The close sequence (`MT_CLEANUP`, `MT_FINISHED`, `MT_CLOSE`) follows `MT_DONE` and reaches a terminal phase that rejects further run actions. The error-close path serializes a UDF failure into the standard close path with the error string prefixed `F-UDF-CL-RUST-` followed by a numeric code. A response whose `message_type` is not valid for the current phase — including an `MT_CALL` received while in a scalar/set run phase — MUST be surfaced as a `ProtocolError` rather than a panic or socket I/O.

## Scenarios

### Scenario: Scalar run loop drives NEXT and EMIT to DONE

* *GIVEN* a `Protocol` past the handshake with `iter_type = ExactlyOnce`
* *WHEN* the host issues `HostAction::SendNext`, the DB replies with an input batch, the host issues `HostAction::Emit`, and finally the host issues `HostAction::Done`
* *THEN* `SendNext` MUST produce an `MT_NEXT` request
* *AND* an input-batch response MUST produce a `HostEvent::InputRows` exposing the column-oriented batch
* *AND* `Emit` MUST produce an `MT_EMIT` request carrying the output column blocks
* *AND* `Done` MUST produce an `MT_DONE` request

### Scenario: Set/EMITS run loop iterates multiple input batches

* *GIVEN* a `Protocol` past the handshake with `iter_type = Multiple`
* *WHEN* the DB delivers more than one input batch before exhaustion
* *THEN* each `HostAction::SendNext` MUST produce an `MT_NEXT` request
* *AND* each non-empty input batch MUST produce a `HostEvent::InputRows`
* *AND* an exhausted-input signal MUST be surfaced so the host can stop iterating and issue `MT_DONE`

### Scenario: Close sequence completes after DONE

* *GIVEN* a `Protocol` that has sent `MT_DONE`
* *WHEN* the DB sends `MT_CLEANUP`, then `MT_FINISHED`, then `MT_CLOSE`
* *THEN* the protocol MUST surface `HostEvent::Cleanup`, `HostEvent::Finished`, and `HostEvent::Close` in that order
* *AND* the protocol MUST reach a terminal phase that rejects further run actions

### Scenario: Ping-pong is echoed immediately

* *GIVEN* a `Protocol` in any non-terminal phase
* *WHEN* the DB sends `MT_PING_PONG`
* *THEN* the protocol MUST surface `HostEvent::PingPong`
* *AND* the corresponding host action MUST produce an `MT_PING_PONG` echo request without advancing the run phase

### Scenario: Reset restarts input iteration

* *GIVEN* a `Protocol` mid-iteration with consumed input batches
* *WHEN* the DB sends `MT_RESET`
* *THEN* the protocol MUST surface `HostEvent::Reset`
* *AND* a subsequent `HostAction::SendNext` MUST resume input iteration from the beginning

### Scenario: Try-again is surfaced for host backoff

* *GIVEN* a `Protocol` awaiting an input batch
* *WHEN* the DB sends `MT_TRY_AGAIN`
* *THEN* the protocol MUST surface `HostEvent::TryAgain`
* *AND* it MUST NOT advance the run phase so the host can re-poll after a backoff

### Scenario: Unexpected message in a phase is a protocol error

* *GIVEN* a `Protocol` in any phase
* *WHEN* a response arrives whose `message_type` is not valid for the current phase, including an `MT_CALL` received while in a scalar/set run phase
* *THEN* the state machine MUST return a `ProtocolError` identifying the unexpected `message_type`
* *AND* it MUST NOT panic or perform socket I/O

### Scenario: Error close path carries the UDF error string

* *GIVEN* a `Protocol` mid-run when the host reports a UDF failure
* *WHEN* the host issues a close-with-error action carrying an error message
* *THEN* the protocol MUST emit a request that serializes the error string into the standard close path
* *AND* the error string MUST be prefixed with `F-UDF-CL-RUST-` followed by a numeric code
