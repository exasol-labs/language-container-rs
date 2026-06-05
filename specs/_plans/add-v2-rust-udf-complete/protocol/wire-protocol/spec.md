# Feature: wire-protocol

Implements the `localzmq+protobuf` wire protocol between the Exasol database and the Rust SLC as a ZMQ DEALER transport plus a pure, I/O-free state machine that translates database responses into host events and host actions into database requests.

## Background

The database acts as a ZMQ ROUTER; the client (`exa-zmq-protocol`) opens a DEALER socket to `ipc://<socket_path>`. Each protobuf message is a single ZMQ frame. The state machine MUST be pure — it consumes decoded `ExascriptResponse` values and produces `ExascriptRequest` values and `HostEvent`s without performing any socket I/O, so it can be unit-tested with fixtures.

v2 extends the protocol with the single-call path (`MT_CALL`, `MT_RETURN`, `MT_UNDEFINED_CALL`) carrying a `SingleCallFunctionId`, and surfaces the `ExascriptConnectionInformationRep` credentials from the handshake info response for connect-back. The error close path continues to use the prefix `F-UDF-CL-RUST-####`.

## Scenarios

<!-- NEW -->
### Scenario: Single-call request surfaces a SingleCall host event

* *GIVEN* a `Protocol` driven past the handshake where `MT_META` carried `single_call_function_id != SC_FN_NIL`
* *WHEN* the database sends an `MT_CALL` response carrying a `single_call_function_id` and its payload
* *THEN* the state machine MUST emit a `HostEvent::SingleCall` carrying the decoded `SingleCallFn` and the call arguments
* *AND* it MUST NOT emit any scalar/set run events (`HostEvent::Next` or `HostEvent::Run`) for that exchange
* *AND* the state machine MUST remain pure, performing no socket I/O
<!-- /NEW -->

<!-- NEW -->
### Scenario: Single-call return is serialized to MT_RETURN

* *GIVEN* a `Protocol` that has emitted a `HostEvent::SingleCall`
* *WHEN* the host supplies a `HostAction::SingleCallReturn` carrying a result payload string
* *THEN* the next `next_request` MUST emit an `MT_RETURN` request whose body carries the result payload
* *AND* the protocol MUST then advance to the close sequence
<!-- /NEW -->

<!-- NEW -->
### Scenario: Unimplemented single-call hook is serialized to MT_UNDEFINED_CALL

* *GIVEN* a `Protocol` that has emitted a `HostEvent::SingleCall`
* *WHEN* the host supplies a `HostAction::UndefinedCall`
* *THEN* the next `next_request` MUST emit an `MT_UNDEFINED_CALL` request
* *AND* the protocol MUST then advance to the close sequence
<!-- /NEW -->

<!-- NEW -->
### Scenario: Connection information is surfaced from the handshake info response

* *GIVEN* a fresh `Protocol` in its initial phase
* *WHEN* the `MT_INFO` response carries an `ExascriptConnectionInformationRep` with host, port, user, and password
* *THEN* the `HostEvent::Info` MUST carry the decoded connection information alongside the script source and connection id
* *AND* a missing `ExascriptConnectionInformationRep` MUST yield `HostEvent::Info` with the connection information absent rather than a protocol error
<!-- /NEW -->

<!-- CHANGED -->
### Scenario: Unexpected message in a phase is a protocol error

* *GIVEN* a `Protocol` in any phase
* *WHEN* a response arrives whose `message_type` is not valid for the current phase, including an `MT_CALL` received while in a scalar/set run phase
* *THEN* the state machine MUST return a `ProtocolError` identifying the unexpected `message_type`
* *AND* it MUST NOT panic or perform socket I/O
<!-- /CHANGED -->
