# Feature: wire-protocol

Implements the `localzmq+protobuf` wire protocol between the Exasol database and the Rust SLC as a ZMQ REQ transport plus a pure, I/O-free state machine that translates database responses into host events and host actions into database requests.

## Background

<!-- DELTA:CHANGED -->
The database acts as a ZMQ `REP` socket; the client (`exa-zmq-protocol`) opens a `REQ` socket to `ipc://<socket_path>`. Each protobuf message is a single ZMQ frame; the `REQ` socket manages the request/reply delimiter automatically, so the client neither writes nor strips an empty delimiter frame. The state machine MUST be pure — it consumes decoded `ExascriptResponse` values and produces `ExascriptRequest` values and `HostEvent`s without performing any socket I/O, so it can be unit-tested with fixtures.
<!-- /DELTA:CHANGED -->

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: REQ transport connects to the IPC socket

* *GIVEN* a valid `ipc://<socket_path>` address
* *WHEN* `ZmqTransport::connect` is called with that path
* *THEN* it MUST open a ZMQ `REQ` socket connected to the address
* *AND* it MUST return a transport whose `send` accepts an `ExascriptRequest` and whose `recv` returns a decoded `ExascriptResponse`
* *AND* a connection failure MUST return a `ProtocolError` rather than panic
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: Transport round-trips a request and response over one frame each

* *GIVEN* a connected `ZmqTransport` paired with a fake `REP` peer
* *WHEN* the client sends an `ExascriptRequest` and the peer replies with one `ExascriptResponse` frame
* *THEN* `send` MUST serialize the request to a single prost-encoded ZMQ frame and MUST NOT prepend an empty delimiter frame, because the `REQ` socket inserts the request/reply delimiter automatically
* *AND* `recv` MUST decode exactly one frame into an `ExascriptResponse` without discarding any delimiter frame, because the `REQ` socket strips the delimiter automatically before delivering the payload
<!-- /DELTA:CHANGED -->
