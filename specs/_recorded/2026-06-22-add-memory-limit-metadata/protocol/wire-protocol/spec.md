# Feature: wire-protocol

Implements the `localzmq+protobuf` wire protocol between the Exasol database and the Rust SLC as a ZMQ REQ transport plus a pure, I/O-free state machine that translates database responses into host events and host actions into database requests.

## Background

The database acts as a ZMQ `REP` socket; the client (`exa-zmq-protocol`) opens a `REQ` socket to `ipc://<socket_path>`. Each protobuf message is a single ZMQ frame; the `REQ` socket manages the request/reply delimiter automatically, so the client neither writes nor strips an empty delimiter frame. The state machine MUST be pure — it consumes decoded `ExascriptResponse` values and produces `ExascriptRequest` values and `HostEvent`s without performing any socket I/O, so it can be unit-tested with fixtures.

The handshake `Info` response (`exascript_info`) carries the per-UDF-instance resident-memory limit in `maximal_memory_limit` (field 11, `required uint64`, bytes), which the database enforces on each VM. `UdfMeta` surfaces this alongside the existing `node_count`/`node_id` and connection-information fields so UDF code can size in-process memory to the sandbox limit.

## Scenarios

<!-- DELTA:NEW -->
### Memory limit is surfaced from the handshake info response

* *GIVEN* a fresh `Protocol` in its initial phase
* *WHEN* the `Info` response carries an `exascript_info` whose `maximal_memory_limit` field is set to a non-zero byte count
* *THEN* the `HostEvent::Meta` MUST carry that value on `UdfMeta::maximal_memory_limit` as an unsigned byte count, decoded verbatim from `exascript_info.maximal_memory_limit` (field 11, `required uint64`), alongside the script source, connection id, and `node_count`
* *AND* the value MUST be interpreted as the per-UDF-instance resident-memory limit in bytes that the database enforces, and MUST NOT be rescaled into any other unit
* *AND* because the proto field is `required`, an `Info` response that omits it MUST yield `UdfMeta::maximal_memory_limit` of `0` (the proto default, denoting "no limit reported") rather than a protocol error
<!-- /DELTA:NEW -->
