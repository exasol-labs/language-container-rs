# Decisions: fix-zmq-req-socket

## ADR: Switch the client ZMQ transport from DEALER to REQ to match the database's REP socket

**ID:** zmq-transport-dealer-to-req
**Plan:** `fix-zmq-req-socket`
**Status:** Accepted

### Context

The database binds a `REP` socket, as the Python3 SLC reference (`exasol/script-languages-release`) confirms. A `REP` peer enforces strict request/reply alternation with exactly one payload frame and does not speak the `DEALER`/`ROUTER` multi-frame envelope.

### Decision

`ZmqTransport::connect` uses `zmq::REQ`. The `REQ` socket manages the request/reply delimiter: `send` writes one payload frame and `recv` reads one payload frame.

### Options Considered

| Option | Verdict |
|--------|---------|
| `zmq::REQ` | ✓ Chosen, canonical `REP` counterpart |
| `DEALER` with manual empty-delimiter framing | ✗ `REP` does not accept the `DEALER` envelope |
| `DEALER` with explicit delimiter sent to `REP` | ✗ Fragile, non-idiomatic |

### Consequences

`send` and `recv` handle no delimiter frame. Transport integration tests mock the DB with a `zmq::REP` peer. The Docker-gated `db_roundtrip` test exercises the full `REQ`/`REP` exchange.
