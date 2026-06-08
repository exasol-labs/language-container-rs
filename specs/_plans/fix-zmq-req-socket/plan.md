# Plan: fix-zmq-req-socket

## Summary

Switch the UDF client's ZMQ transport from a `DEALER` socket to a `REQ` socket so it matches the database's `REP` socket and honors the strict request/reply alternation the DB enforces, removing the manual empty-delimiter framing that `DEALER` required.

## Design

Small, well-scoped correctness fix to a single transport module. No ADR-level design is required; the consequences table below records the socket-type decision.

### Context

The Exasol architect confirmed the database binds a `REP` socket (not `ROUTER`), and confirmed this against the Python3 SLC reference implementation (https://github.com/exasol/script-languages-release). A `REP` peer enforces strict request/reply alternation and delivers/expects exactly one payload frame; it does not carry routing identities or speak the `DEALER`/`ROUTER` multi-frame envelope. The current `DEALER` client manually inserts and strips an empty delimiter frame to imitate `DEALER`→`ROUTER` framing — this is the wrong wire shape for a `REP` peer and relies on asynchronous send/recv semantics the DB does not support.

- **Goals** — make the client speak the same socket pattern as the DB (`REQ`↔`REP`); remove hand-rolled delimiter framing; keep `send`/`recv` delivering exactly one prost-encoded frame each.
- **Non-Goals** — no change to the pure protocol state machine, message schema, or any `HostEvent`/`HostAction`; no change to the IPC endpoint format.

### Decision

Use `zmq::REQ` in `ZmqTransport::connect`. Let the `REQ` socket manage the request/reply delimiter automatically: `send` writes a single payload frame, `recv` reads a single payload frame. The DB's `REP` socket mirrors this exactly.

#### Patterns

| Pattern | Where | Why |
|---------|-------|-----|
| Synchronous REQ↔REP lock-step | `ZmqTransport` | Matches the DB's `REP` socket; enforces alternation the protocol state machine already assumes |
| Library-managed delimiter | `send` / `recv` | `REQ` inserts and strips the empty delimiter; removes error-prone manual framing |

### Consequences

| Decision | Alternatives Considered | Rationale |
|----------|------------------------|-----------|
| Use `zmq::REQ` | Keep `DEALER` with manual framing; use `DEALER` with explicit delimiter to a `REP` peer | `REP` peers reject the `DEALER` envelope shape; `REQ` is the canonical, lock-step counterpart and removes manual framing bugs. Confirmed against the Python3 SLC reference. |

## Features

| Feature | Status | Spec |
|---------|--------|------|
| wire-protocol | CHANGED | `specs/_plans/fix-zmq-req-socket/protocol/wire-protocol/spec.md` |

## Migration

The permanent `wire-protocol` feature Background text still describes the transport as `DEALER` against a DB `ROUTER`. When this plan is recorded, the Background MUST be updated to describe a `REQ` transport against a DB `REP` socket.

| Current (Background) | New (Background) |
|----------------------|------------------|
| "as a ZMQ DEALER transport" | "as a ZMQ REQ transport" |
| "The database acts as a ZMQ ROUTER; the client opens a DEALER socket" | "The database acts as a ZMQ REP socket; the client opens a REQ socket" |

## Implementation Tasks

1. In `crates/exa-zmq-protocol/src/transport.rs`, change `ctx.socket(zmq::DEALER)` to `ctx.socket(zmq::REQ)` in `connect`, and rewrite the doc comment to describe REQ↔REP lock-step framing.
2. In `send`, remove the `self.socket.send(b"" as &[u8], zmq::SNDMORE)?` empty-delimiter frame so only the single prost payload frame is sent; update the doc comment.
3. In `recv`, remove the `let _ = self.socket.recv_bytes(0)?;` empty-delimiter discard so the first received frame is decoded directly; update the doc comment.
4. In `crates/exa-zmq-protocol/tests/transport.rs`, replace the `zmq::ROUTER` mock peer with a `zmq::REP` peer in both tests: `transport_connects_to_ipc` binds a `REP` socket; `transport_round_trip_single_frame` does a single `recv_bytes`/decode then `encode`/`send` of one frame, dropping all identity- and delimiter-frame handling. Rename the connect test's assertion message to reference REQ→REP.
5. Run `cargo build --release`, `cargo test -p exa-zmq-protocol`, and `cargo fmt --check && cargo clippy --all-targets --all-features -- -D warnings`; confirm the transport tests pass and no warnings remain. [expert]

## Parallelization

| Parallel Group | Tasks |
|----------------|-------|
| Group A | Task 1, Task 2, Task 3 (same file, do together) |
| Group B | Task 4 |

Sequential dependencies:
- Group A → Group B → Task 5 (verification depends on both source and test changes)

## Dead Code Removal

| Type | Location | Reason |
|------|----------|--------|
| Statement | `crates/exa-zmq-protocol/src/transport.rs` `send` | Manual empty-delimiter `SNDMORE` frame no longer needed under REQ |
| Statement | `crates/exa-zmq-protocol/src/transport.rs` `recv` | Empty-delimiter discard no longer needed under REQ |
| Test scaffolding | `crates/exa-zmq-protocol/tests/transport.rs` | ROUTER identity/delimiter envelope handling replaced by plain REP recv/send |

## Verification

### Scenario Coverage

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| REQ transport connects to the IPC socket | Integration | `crates/exa-zmq-protocol/tests/transport.rs` | `transport_connects_to_ipc` |
| Transport round-trips a request and response over one frame each | Integration | `crates/exa-zmq-protocol/tests/transport.rs` | `transport_round_trip_single_frame` |

End-to-end proof that the REQ↔REP wire shape works against a live DB is provided by the existing db-roundtrip integration test `crates/it/tests/db_roundtrip.rs::db_roundtrip_all_scenarios` (gated on Docker), which drives `MT_CLIENT` and the full run loop through the real `REP` socket.

### Manual Testing

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| wire-protocol | `cargo test -p exa-zmq-protocol --test transport` | Both transport tests pass; output shows `test result: ok` |
| wire-protocol | `cargo test -p it --features integration db_roundtrip -- --nocapture` (Docker available) | `[it] SELECT 1 ok` then each scenario logs `ok`; no post-`MT_CLIENT` hang |

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Build | `cargo build --release` | Exit 0 |
| Test | `cargo test` | 0 failures |
| Lint | `cargo fmt --check && cargo clippy --all-targets --all-features -- -D warnings` | 0 errors/warnings |
| Format | `cargo fmt` | No changes |
