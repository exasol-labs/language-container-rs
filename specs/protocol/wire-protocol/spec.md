# Feature: wire-protocol

Implements the `localzmq+protobuf` wire protocol between the Exasol database and the Rust SLC as a ZMQ REQ transport plus a pure, I/O-free state machine that translates database responses into host events and host actions into database requests.

## Background

The database acts as a ZMQ `REP` socket; the client (`exa-zmq-protocol`) opens a `REQ` socket to `ipc://<socket_path>`. Each protobuf message is a single ZMQ frame; the `REQ` socket manages the request/reply delimiter automatically, so the client neither writes nor strips an empty delimiter frame. The state machine MUST be pure — it consumes decoded `ExascriptResponse` values and produces `ExascriptRequest` values and `HostEvent`s without performing any socket I/O, so it can be unit-tested with fixtures.

v2 extends the protocol with the single-call path (`MT_CALL`, `MT_RETURN`, `MT_UNDEFINED_CALL`) carrying a `SingleCallFunctionId`, and surfaces the `ExascriptConnectionInformationRep` credentials from the handshake info response for connect-back. The error close path continues to use the prefix `F-UDF-CL-RUST-####`. In single-call mode the DB acknowledges the container's `MT_RETURN` result by echoing `MT_RETURN`; the state machine surfaces this as `HostEvent::SingleCallAck` so the dispatch loop can close the run with `MT_DONE`. In non-single-call mode, `MT_RETURN` in the run phase remains a protocol error.

The handshake `Info` response (`exascript_info`) carries the per-UDF-instance resident-memory limit in `maximal_memory_limit` (field 11, `required uint64`, bytes), which the database enforces on each VM. `UdfMeta` surfaces this alongside the existing `node_count`/`node_id` and connection-information fields so UDF code can size in-process memory to the sandbox limit.

## Scenarios

### Scenario: REQ transport connects to the IPC socket

* *GIVEN* a valid `ipc://<socket_path>` address
* *WHEN* `ZmqTransport::connect` is called with that path
* *THEN* it MUST open a ZMQ `REQ` socket connected to the address
* *AND* it MUST return a transport whose `send` accepts an `ExascriptRequest` and whose `recv` returns a decoded `ExascriptResponse`
* *AND* a connection failure MUST return a `ProtocolError` rather than panic

### Scenario: Transport round-trips a request and response over one frame each

* *GIVEN* a connected `ZmqTransport` paired with a fake `REP` peer
* *WHEN* the client sends an `ExascriptRequest` and the peer replies with one `ExascriptResponse` frame
* *THEN* `send` MUST serialize the request to a single prost-encoded ZMQ frame and MUST NOT prepend an empty delimiter frame, because the `REQ` socket inserts the request/reply delimiter automatically
* *AND* `recv` MUST decode exactly one frame into an `ExascriptResponse` without discarding any delimiter frame, because the `REQ` socket strips the delimiter automatically before delivering the payload

### Scenario: Handshake exchange produces Info then Meta events

* *GIVEN* a fresh `Protocol` in its initial phase
* *WHEN* the protocol is driven from start with an `MT_INFO` response followed by an `MT_META` response
* *THEN* the first `next_request` MUST emit an `MT_CLIENT` request
* *AND* on the `MT_INFO` response it MUST emit a `HostEvent::Info` carrying the script source and connection id, then issue an `MT_META` request
* *AND* on the `MT_META` response it MUST emit a `HostEvent::Meta` carrying the column definitions and `iter_type`

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

### Scenario: Connection information is surfaced from the handshake info response

* *GIVEN* a fresh `Protocol` in its initial phase
* *WHEN* the `MT_INFO` response carries an `ExascriptConnectionInformationRep` with host, port, user, and password
* *THEN* the `HostEvent::Info` MUST carry the decoded connection information alongside the script source and connection id
* *AND* a missing `ExascriptConnectionInformationRep` MUST yield `HostEvent::Info` with the connection information absent rather than a protocol error

### Scenario: MT_RETURN DB acknowledgement in single-call mode surfaces SingleCallAck

* *GIVEN* a `Protocol` in single-call mode where `single_call_mode = true`
* *WHEN* the DB sends `MT_RETURN` in the Run phase (acknowledging the container's `MT_RETURN` result)
* *THEN* the state machine MUST emit `HostEvent::SingleCallAck` and MUST NOT treat this as a protocol error
* *AND* in a non-single-call protocol, an `MT_RETURN` received in the Run phase MUST still be surfaced as a `ProtocolError::UnexpectedMessage`

### Scenario: Memory limit is surfaced from the handshake info response

* *GIVEN* a fresh `Protocol` in its initial phase
* *WHEN* the `Info` response carries an `exascript_info` whose `maximal_memory_limit` field is set to a non-zero byte count
* *THEN* the `HostEvent::Meta` MUST carry that value on `UdfMeta::maximal_memory_limit` as an unsigned byte count, decoded verbatim from `exascript_info.maximal_memory_limit` (field 11, `required uint64`), alongside the script source, connection id, and `node_count`
* *AND* the value MUST be interpreted as the per-UDF-instance resident-memory limit in bytes that the database enforces, and MUST NOT be rescaled into any other unit
* *AND* because the proto field is `required`, an `Info` response that omits it MUST yield `UdfMeta::maximal_memory_limit` of `0` (the proto default, denoting "no limit reported") rather than a protocol error

### Scenario: Transient EAGAIN on recv/send is retried until the 120 s backstop

* *GIVEN* a connected `ZmqTransport` whose `RCVTIMEO`/`SNDTIMEO` is set to 1 s (a poll interval, not a deadline)
* *WHEN* `recv` or `send` returns a ZMQ `EAGAIN` error because the 1 s poll interval elapsed before a frame arrived or was queued
* *THEN* the transport MUST retry the operation rather than propagating the `EAGAIN` as a fatal error, preserving the REQ/REP lock-step exchange
* *AND* retries MUST continue as long as the total elapsed time since the call began is less than 120 s (`MAX_TOTAL_WAIT`)
* *AND* once 120 s of continuous `EAGAIN` responses have elapsed, the transport MUST return a timeout `ProtocolError` to the caller
* *AND* any non-`EAGAIN` socket error (genuine failure) MUST propagate immediately without waiting for the backstop
