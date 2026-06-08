# Tasks: fix-zmq-req-socket

## Phase 3: Implementation (Group A — transport.rs source changes)
- [x] 1. Change `ctx.socket(zmq::DEALER)` to `ctx.socket(zmq::REQ)` in `connect`; rewrite doc comment to describe REQ↔REP lock-step framing
- [x] 2. Remove `self.socket.send(b"" as &[u8], zmq::SNDMORE)?` empty-delimiter frame from `send`; update doc comment
- [x] 3. Remove `let _ = self.socket.recv_bytes(0)?;` empty-delimiter discard from `recv`; update doc comment

## Phase 3: Implementation (Group B — test file changes)
- [x] 4. Replace `zmq::ROUTER` mock peer with `zmq::REP` peer in both transport tests; simplify recv/send to single frame; rename assertion messages to reference REQ→REP

## Phase 5: Verification
- [x] 5. Run `cargo build --release`, `cargo test -p exa-zmq-protocol`, lint/fmt checks; confirm transport tests pass and no warnings remain [expert]
