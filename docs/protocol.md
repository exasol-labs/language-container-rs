[language-container-rs](../README.md) · [Docs](index.md)

# The Exasol UDF protocol

This is how an Exasol database node and the Rust script-language client
(`exaudfclient`) talk to each other while a UDF runs. It is the wire protocol
the runtime implements in [`crates/exa-zmq-protocol`](../crates/exa-zmq-protocol)
and [`crates/exa-udf-runtime/src/dispatch.rs`](../crates/exa-udf-runtime/src/dispatch.rs);
the message definitions live in
[`crates/exa-proto/proto/zmqcontainer.proto`](../crates/exa-proto/proto/zmqcontainer.proto).

> **Connect-back is NOT part of this protocol.** The control channel below only
> drives UDF execution. When a UDF "connects back" to the database it opens a
> *separate, ordinary SQL login* (via exarrow-rs) — see the
> [connect-back guide](writing-a-udf.md#8-connect-back). Don't conflate the two.

## Transport: one ZMQ REQ/REP socket

The database launches the language container and hands the client a single
endpoint as `argv[1]`:

- **`ipc://…`** on single-node Docker (a Unix-domain socket file).
- **`tcp://<node-ip>:<port>`** on a multi-node cluster.

The client connects a **ZMQ `REQ`** socket; the database binds the matching
`REP` socket. REQ/REP enforces strict lock-step: the client sends exactly one
request, waits for exactly one reply, then may send again. Every message is a
single protobuf-encoded frame (`ExascriptRequest` out, `ExascriptResponse` in).
Socket options match the reference libexaudflib: `LINGER=0`, `RCVTIMEO=1000ms`,
`SNDTIMEO=1000ms`.

The transport choice (IPC vs TCP) is the database's call at launch and has **no
bearing** on connect-back, which is always a plain TCP login to the SQL port.

## Message types

From `zmqcontainer.proto` (`message_type`):

| # | Type | Meaning |
|---|------|---------|
| 1 | `MT_CLIENT` | client announces itself (carries `client_name`) |
| 2 | `MT_INFO` | DB → client: session/runtime info |
| 3 | `MT_META` | column metadata exchange |
| 4 | `MT_CLOSE` | abnormal close (carries an exception message) |
| 5 | `MT_IMPORT` | fetch a named CONNECTION's credentials |
| 6 | `MT_NEXT` | client asks for the next input batch |
| 7 | `MT_RESET` | restart input iteration (set UDFs) |
| 8 | `MT_EMIT` | client ships an output batch |
| 9 | `MT_RUN` | open the run phase |
| 10 | `MT_DONE` | input exhausted for this run |
| 11 | `MT_CLEANUP` | DB tells the client to tear down |
| 12 | `MT_FINISHED` | clean end-of-session handshake |
| 13 | `MT_PING_PONG` | liveness ping (echoed back) |
| 14 | `MT_TRY_AGAIN` | transient retry signal |
| 15–17 | `MT_CALL` / `MT_RETURN` / `MT_UNDEFINED_CALL` | single-call mode (import/export specs, virtual schema) |

## Lifecycle

The runtime drives a three-phase state machine (`Protocol` in
[`loop_.rs`](../crates/exa-zmq-protocol/src/loop_.rs), loop in
[`dispatch.rs`](../crates/exa-udf-runtime/src/dispatch.rs)):

**1. Handshake.** Client sends `MT_CLIENT`; DB replies `MT_INFO`; client sends
`MT_META`; DB replies `MT_META` with the input/output column types. The runtime
resolves the `%udf_object` path, loads the `.so`, and validates the annotated
schema against the DB's column metadata.

**2. Run** (repeats per group; one group for scalar/DUAL, N for `SET`):
```
client → MT_RUN
  DB → MT_RUN                       (open) | MT_CLEANUP (no more groups)
  loop:
    client → MT_NEXT
      DB → MT_NEXT(table)           one input batch  → run the UDF over it
      DB → MT_DONE                  input exhausted  → break
    client → MT_EMIT(table)         flush this batch's output (if any)
      DB → MT_EMIT                  ack
  client → MT_DONE
  DB → MT_DONE                      (echo) | MT_CLEANUP
```
A `MT_PING_PONG` may arrive at any point and is echoed transparently while the
REQ socket stays in lock-step.

**3. Cleanup.** When the DB answers an `MT_RUN` with `MT_CLEANUP`, the client
replies `MT_FINISHED`; the DB echoes `MT_FINISHED` and the session ends. On a
UDF error the client sends `MT_CLOSE` carrying an `F-UDF-CL-RUST-####` message.

The client then exits the process. (`exaudfclient` calls `std::process::exit(0)`
on success so the DB's `waitpid` reaps it promptly — see
[`crates/exaudfclient/src/main.rs`](../crates/exaudfclient/src/main.rs).)

## Fetching CONNECTION credentials (`MT_IMPORT`)

When a UDF calls `ctx.connection("NAME")`, the runtime issues an `MT_IMPORT`
request (`PB_IMPORT_CONNECTION_INFORMATION`) for that name and the DB returns the
stored `address` / `user` / `password` as a `ConnectionObject`. This is the only
place credentials cross the control channel; the UDF then uses them to open a
**separate** SQL session (connect-back). The exchange happens while the dispatch
loop is blocked inside `run()`, so the socket is idle and the round-trip is safe.

## Column data encoding

Input/output batches are columnar (`ExascriptTableData`): per-type blocks
(`data_int64`, `data_double`, `data_string`, `data_bool`, …) plus a `data_nulls`
bitmap. The runtime maps these to the SDK `Value` enum. **Exasol `BIGINT` and
`DECIMAL` travel in the *string* block as `PB_NUMERIC`** — emit them as
`Value::Numeric` (now carrying a `Decimal`), not `Value::Int64` (see the
write-back guide's Pitfalls).

The wire delivers each column as one of eight proto column types.
`ColumnMeta::from_pb` refines those using the SQL-level `type_name` field into
the canonical `ExaType` enum, which now lives in `exasol_udf_sdk::value` and is
re-exported by `exa-zmq-protocol`.
