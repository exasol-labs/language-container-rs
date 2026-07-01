# Feature: wire-protocol

Implements the `localzmq+protobuf` wire protocol between the Exasol database and the Rust SLC as a ZMQ REQ transport plus a pure, I/O-free state machine that translates database responses into host events and host actions into database requests.

## Background

The database acts as a ZMQ `REP` socket; the client (`exa-zmq-protocol`) opens a `REQ` socket to `ipc://<socket_path>`. Each protobuf message is a single ZMQ frame. The state machine is pure — it consumes decoded `ExascriptResponse` values and produces `ExascriptRequest` values and `HostEvent`s without performing any socket I/O.

The handshake surfaces `exascript_info` metadata on `UdfMeta` (session/node/vm identity, the memory limit, and DB/script/user fields). Connect-back credentials, by contrast, are resolved on demand per CONNECTION name via the live `MT_IMPORT` exchange and are NOT buffered onto `UdfMeta`.

## Scenarios

<!-- DELTA:NEW -->
### Scenario: Handshake metadata carries no buffered connect-back credentials

* *GIVEN* a `Protocol` completing the handshake where the DB may deliver a `HostEvent::ConnInfo` before the `MT_META` that ends the handshake
* *WHEN* the host assembles the `UdfMeta` surfaced by `HostEvent::Meta`
* *THEN* `UdfMeta` MUST NOT carry a buffered connection-information field, because connect-back credentials are resolved on demand per CONNECTION name via the live `MT_IMPORT` exchange rather than captured during the handshake
* *AND* the `ConnInfo` type and the `HostEvent::ConnInfo` event MUST remain available, because the on-demand connect-back path still decodes credentials through them
* *AND* the handshake loop MUST NOT buffer a `HostEvent::ConnInfo` into `UdfMeta`, leaving the on-demand resolver as the single credential path
<!-- /DELTA:NEW -->
