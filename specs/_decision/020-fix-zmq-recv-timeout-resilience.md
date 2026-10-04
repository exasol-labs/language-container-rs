# Decisions: fix-zmq-recv-timeout-resilience

## ADR: ZmqTransport retries EAGAIN on recv/send up to a 120 s backstop

**ID:** zmqtransport-retries-eagain-120s-backstop
**Plan:** `fix/zmq-recv-timeout-resilience`
**Status:** Accepted

### Context

`RCVTIMEO`/`SNDTIMEO` on the ZMQ `REQ` socket is 1 s and acts as a poll interval: ZMQ returns `EAGAIN` when no frame is ready. A loaded database can legitimately reply later, and treating `EAGAIN` as fatal aborts the UDF and breaks REQ/REP lock-step.

### Decision

`ZmqTransport` wraps `send` and `recv` in `retry_transient`. It re-issues the call on `EAGAIN` until a non-`EAGAIN` result arrives or `MAX_TOTAL_WAIT` (120 s) of continuous `EAGAIN` has elapsed. Every other ZMQ error is a fatal `ProtocolError`. The `is_transient` predicate holds the retry classification.

### Options Considered

| Option | Verdict |
|--------|---------|
| Retry `EAGAIN` up to 120 s | ✓ Chosen |
| Treat `EAGAIN` as fatal | ✗ Aborts the UDF on a slow but live database |
| Raise `RCVTIMEO`/`SNDTIMEO` | ✗ Detects a hung connection only after a long wait |

### Consequences

The wire-protocol spec documents the retry bounds.
