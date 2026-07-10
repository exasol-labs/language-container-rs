# Decisions: fix-zmq-req-socket

## ADR: Switch the client ZMQ transport from DEALER to REQ to match the database's REP socket

**ID:** zmq-transport-dealer-to-req
**Plan:** `fix-zmq-req-socket`
**Status:** Accepted

### Context

The UDF client transport (`ZmqTransport`) was opening a `DEALER` socket and manually inserting an empty delimiter frame on `send` and discarding one on `recv` to imitate the `DEALER`/`ROUTER` multi-frame envelope. The Exasol architect confirmed the database actually binds a `REP` socket — not `ROUTER` — and this was validated against the Python3 SLC reference implementation (`exasol/script-languages-release`). A `REP` peer enforces strict request/reply alternation and delivers/expects exactly one payload frame; it does not carry routing identities or speak the `DEALER`/`ROUTER` multi-frame envelope. The `DEALER` client was using the wrong wire shape and relied on asynchronous send/recv semantics the DB does not support.

### Decision

Use `zmq::REQ` in `ZmqTransport::connect`. Let the `REQ` socket manage the request/reply delimiter automatically: `send` writes a single payload frame, `recv` reads a single payload frame. The DB's `REP` socket mirrors this exactly.

### Options Considered

| Option | Verdict |
|--------|---------|
| Use `zmq::REQ` (canonical `REP` counterpart) | ✓ Chosen — `REP` peers reject the `DEALER` envelope shape; `REQ` is the canonical, lock-step counterpart and removes manual framing bugs. Confirmed against the Python3 SLC reference. |
| Keep `DEALER` with manual empty-delimiter framing | ✗ Rejected — `REP` does not speak the `DEALER`/`ROUTER` multi-frame envelope; the manual delimiter insertion was the root cause of the post-`MT_CLIENT` hang |
| Use `DEALER` with an explicit delimiter sent to a `REP` peer | ✗ Rejected — fragile and non-idiomatic; `REQ` is the canonical counterpart and removes all hand-rolled framing |

### Consequences

The `send` implementation no longer prepends an empty delimiter frame; the `recv` implementation no longer discards one. The `REQ` socket enforces lock-step alternation that the protocol state machine already assumes. The transport integration tests now mock the DB with a `zmq::REP` peer, matching the real wire shape. The end-to-end `db_roundtrip` integration test (gated on Docker) exercises the full `REQ`/`REP` exchange against the live DB.
