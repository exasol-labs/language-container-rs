# Decisions: fix-emit-ingest-wire-path

## ADR: Remove the transport wall-clock cap rather than raise it

**ID:** remove-transport-wall-clock-cap
**Plan:** fix-emit-ingest-wire-path
**Status:** Accepted

### Context

Issue #95 flagged the transport's 120 s `MAX_TOTAL_WAIT` backstop on ZMQ `EAGAIN` retries. The Exasol engine's own retry loop (`zmqinternal.cc:903-980`) aborts only on a zombie or missing socket, never on elapsed time. Any finite client-side cap turns a slow query into a VM crash instead of a completed, if slow, result. The database watchdog already ends a wedged session.

### Decision

Delete the `MAX_TOTAL_WAIT` constant and its elapsed-time branch. Retry ZMQ `EAGAIN` indefinitely on `send` and `recv`. Session termination for a genuinely stuck exchange is the database watchdog's responsibility, not the transport's.

### Options Considered

| Option | Verdict |
|--------|---------|
| Remove the cap entirely, rely on the engine watchdog | ✓ Chosen — matches the engine's own unbounded retry behavior and removes a client-side failure mode the engine does not have |
| Raise the cap and log | ✗ Rejected — any finite cap still contradicts the engine's unbounded wait and remains a source of spurious VM crashes under a slow query |

### Consequences

A wedged ZMQ exchange no longer self-terminates from the client side; the database watchdog becomes the sole backstop for a stuck session. The retry loop keeps emitting its per-poll `debug!` progress event so a long wait stays observable at `%udf_debug_level` debug.
