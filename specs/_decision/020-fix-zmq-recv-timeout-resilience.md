# Decisions: fix-zmq-recv-timeout-resilience

## ADR: ZmqTransport retries EAGAIN on recv/send up to a 120 s backstop

**ID:** zmqtransport-retries-eagain-120s-backstop
**Plan:** `fix/zmq-recv-timeout-resilience` (closes #37)
**Status:** Accepted

### Context

The `RCVTIMEO`/`SNDTIMEO` socket option on the ZMQ `REQ` socket is set to 1 s. This is a *poll interval*, not a deadline: ZMQ returns `EAGAIN` when the interval elapses with no frame ready. Under a loaded cluster the database may legitimately take longer than 1 s to respond, so the previous code — which treated `EAGAIN` identically to a hard socket error — aborted the UDF with a spurious `ProtocolError` and broke REQ/REP lock-step.

### Decision

Introduce `retry_transient` in `ZmqTransport`: loop on `EAGAIN`, re-issuing the underlying `send`/`recv` call each iteration, until either a non-`EAGAIN` result arrives or `MAX_TOTAL_WAIT` (120 s) of continuous `EAGAIN` has elapsed. Only `EAGAIN` is retryable; every other ZMQ error propagates immediately as a fatal `ProtocolError`. The 120 s backstop is deliberately far above any plausible under-load reply latency so it catches only genuinely hung connections.

### Options Considered

| Option | Verdict |
|--------|---------|
| Retry `EAGAIN` up to `MAX_TOTAL_WAIT` (120 s) | ✓ Chosen — a slow-but-alive DB no longer aborts the UDF; genuine socket failures still surface immediately; backstop prevents unbounded hangs |
| Propagate `EAGAIN` as fatal immediately (previous behaviour) | ✗ Rejected — treats transient poll-interval expiry identically to a hard socket failure; causes spurious UDF aborts under load |
| Increase `RCVTIMEO`/`SNDTIMEO` to a large value | ✗ Rejected — a large socket timeout means a hung connection is only detected after a very long wall-clock wait with no intermediate progress signals |

### Consequences

`send` and `recv` in `ZmqTransport` now call `retry_transient` instead of invoking the socket directly. The REQ/REP lock-step contract is preserved across slow cluster responses. The `is_transient` predicate keeps the retry classification in one place and is trivially unit-testable. The wire-protocol spec gains a matching scenario documenting the retry bounds.
