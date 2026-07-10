# Feature: handshake

Opens the ZMQ REQ transport to the database's REP socket and drives the `MT_CLIENT`/`MT_INFO`/`MT_META` exchange that bootstraps a UDF session, surfacing connection metadata and the memory limit to the host without buffering connect-back credentials.

## Background

The database acts as a ZMQ `REP` socket; the client (`exa-zmq-protocol`) opens a `REQ` socket to `ipc://<socket_path>`. Each protobuf message is a single ZMQ frame; the `REQ` socket manages the request/reply delimiter automatically, so the client neither writes nor strips an empty delimiter frame. A transient ZMQ `EAGAIN` on `send`/`recv` MUST be retried rather than treated as fatal, bounded by a 120 s backstop (`MAX_TOTAL_WAIT`), preserving the REQ/REP lock-step exchange.

The handshake exchange surfaces `exascript_info` metadata on `UdfMeta` (session/node/vm identity, the memory limit, and DB/script/user fields), and surfaces the `ExascriptConnectionInformationRep` credentials from the handshake info response. Connect-back credentials, by contrast, are resolved on demand per CONNECTION name via the live `MT_IMPORT` exchange and are NOT buffered onto `UdfMeta`.

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

### Scenario: Connection information is surfaced from the handshake info response

* *GIVEN* a fresh `Protocol` in its initial phase
* *WHEN* the `MT_INFO` response carries an `ExascriptConnectionInformationRep` with host, port, user, and password
* *THEN* the `HostEvent::Info` MUST carry the decoded connection information alongside the script source and connection id
* *AND* a missing `ExascriptConnectionInformationRep` MUST yield `HostEvent::Info` with the connection information absent rather than a protocol error

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

### Scenario: Handshake metadata carries no buffered connect-back credentials

* *GIVEN* a `Protocol` completing the handshake where the DB may deliver a `HostEvent::ConnInfo` before the `MT_META` that ends the handshake
* *WHEN* the host assembles the `UdfMeta` surfaced by `HostEvent::Meta`
* *THEN* `UdfMeta` MUST NOT carry a buffered connection-information field, because connect-back credentials are resolved on demand per CONNECTION name via the live `MT_IMPORT` exchange rather than captured during the handshake
* *AND* the `ConnInfo` type and the `HostEvent::ConnInfo` event MUST remain available, because the on-demand connect-back path still decodes credentials through them
* *AND* the handshake loop MUST NOT buffer a `HostEvent::ConnInfo` into `UdfMeta`, leaving the on-demand resolver as the single credential path
