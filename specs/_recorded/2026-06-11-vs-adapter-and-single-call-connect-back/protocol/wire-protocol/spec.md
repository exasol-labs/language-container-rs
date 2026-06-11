# Feature: wire-protocol

Implements the `localzmq+protobuf` wire protocol between the Exasol database and the Rust SLC as a ZMQ REQ transport plus a pure, I/O-free state machine that translates database responses into host events and host actions into database requests.

## Background

The database acts as a ZMQ `REP` socket; the client (`exa-zmq-protocol`) opens a `REQ` socket to `ipc://<socket_path>`. Each protobuf message is a single ZMQ frame. The state machine MUST be pure — it consumes decoded `ExascriptResponse` values and produces `ExascriptRequest` values and `HostEvent`s without performing any socket I/O. In single-call mode the DB acknowledges the container's `MT_RETURN` result by echoing `MT_RETURN`; the state machine surfaces this as `HostEvent::SingleCallAck` so the dispatch loop can close the run with `MT_DONE`. In non-single-call mode, `MT_RETURN` in the run phase remains a protocol error.

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Single-call return is serialized to MT_RETURN

* *GIVEN* a `Protocol` in single-call mode that has emitted a `HostEvent::SingleCall`
* *WHEN* the host supplies a `HostAction::SingleCallReturn` carrying a result payload string
* *THEN* the next `next_request` MUST emit an `MT_RETURN` request whose body carries the result payload
* *AND* when the DB echoes `MT_RETURN` as the acknowledgement, the state machine MUST emit `HostEvent::SingleCallAck` so the dispatch loop can close the run with `MT_DONE`
* *AND* the protocol MUST NOT advance to the close sequence on `MT_RETURN` alone — the session ends only on a subsequent `MT_CLEANUP`
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: MT_RETURN DB acknowledgement in single-call mode surfaces SingleCallAck

* *GIVEN* a `Protocol` in single-call mode where `single_call_mode = true`
* *WHEN* the DB sends `MT_RETURN` in the Run phase (acknowledging the container's `MT_RETURN` result)
* *THEN* the state machine MUST emit `HostEvent::SingleCallAck` and MUST NOT treat this as a protocol error
* *AND* in a non-single-call protocol, an `MT_RETURN` received in the Run phase MUST still be surfaced as a `ProtocolError::UnexpectedMessage`
<!-- /DELTA:NEW -->
