# Feature: handshake

Opens the ZMQ REQ transport to the database's REP socket and drives the `MT_CLIENT`/`MT_INFO`/`MT_META` exchange that bootstraps a UDF session, surfacing connection metadata and the memory limit to the host without buffering connect-back credentials.

## Background

<!-- DELTA:CHANGED -->
The database acts as a ZMQ `REP` socket; the client (`exa-zmq-protocol`) opens a `REQ` socket to `ipc://<socket_path>`. Each protobuf message is a single ZMQ frame; the `REQ` socket manages the request/reply delimiter automatically, so the client neither writes nor strips an empty delimiter frame. A transient ZMQ `EAGAIN` on `send`/`recv` MUST be retried indefinitely, with no wall-clock cap. Session termination is the database watchdog's responsibility.

The transport hands each outbound frame to libzmq as an owned `zmq::Message` and decodes each inbound frame from the received message's slice.
<!-- /DELTA:CHANGED -->

## Scenarios

<!-- DELTA:REMOVED -->
### Scenario: Transient EAGAIN on recv/send is retried until the 120 s backstop

* *GIVEN* a connected `ZmqTransport` whose `RCVTIMEO`/`SNDTIMEO` is set to 1 s (a poll interval, not a deadline)
* *WHEN* `recv` or `send` returns a ZMQ `EAGAIN` error because the 1 s poll interval elapsed before a frame arrived or was queued
* *THEN* this scenario MUST be removed, because its 120 s backstop clause contradicts the engine's unbounded wait and is replaced by the uncapped-retry scenario below
<!-- /DELTA:REMOVED -->

<!-- DELTA:NEW -->
### Scenario: Transient EAGAIN on recv/send is retried without a wall-clock cap

* *GIVEN* a connected `ZmqTransport` whose `RCVTIMEO`/`SNDTIMEO` is set to 1 s (a poll interval, not a deadline)
* *WHEN* `recv` or `send` returns a ZMQ `EAGAIN` error repeatedly because the database has not yet replied or queued the frame
* *THEN* the transport MUST keep retrying for as long as `EAGAIN` continues, with no total-elapsed-time limit, preserving the REQ/REP lock-step exchange
* *AND* the `MAX_TOTAL_WAIT` constant and the timeout `ProtocolError` MUST be removed
* *AND* any non-`EAGAIN` socket error MUST still propagate immediately without retry
* *AND* the retry loop MUST keep emitting its per-poll `debug!` progress event carrying the elapsed wait, so a long wait stays observable at `%udf_debug_level` debug
<!-- /DELTA:NEW -->

<!-- DELTA:CHANGED -->
### Scenario: Transport round-trips a request and response over one frame each

* *GIVEN* a connected `ZmqTransport` paired with a fake `REP` peer
* *WHEN* the client sends an `ExascriptRequest` and the peer replies with one `ExascriptResponse` frame
* *THEN* `send` MUST serialize the request to a single prost-encoded ZMQ frame and MUST NOT prepend an empty delimiter frame, because the `REQ` socket inserts the request/reply delimiter automatically
* *AND* `recv` MUST decode exactly one frame into an `ExascriptResponse` without discarding any delimiter frame, because the `REQ` socket strips the delimiter automatically before delivering the payload
* *AND* `send` MUST pass the encoded frame to libzmq as a `zmq::Message` built from the owned encoding buffer
* *AND* a transient `EAGAIN` retry MUST re-encode the request for the next attempt, since `Socket::send` consumes the message
* *AND* `recv` MUST decode directly from the received `zmq::Message`'s byte slice, not from an intermediate `Vec<u8>` copy
<!-- /DELTA:CHANGED -->
</content>
