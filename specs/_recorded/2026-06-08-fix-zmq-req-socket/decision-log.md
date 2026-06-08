# Decision Log: fix-zmq-req-socket

Date: 2026-06-08

## Interview

**Q:** Has the DB's socket type been confirmed?
**A:** Yes — the DB is a `REP` socket. Confirmed by reference to the Python3 SLC implementation (https://github.com/exasol/script-languages-release).

## Design Decisions

### [1] Switch the client transport from DEALER to REQ

- **Decision:** Use `zmq::REQ` in `ZmqTransport::connect`, and let the socket manage the request/reply delimiter automatically (single payload frame on both `send` and `recv`).
- **Alternatives:** (a) Keep `DEALER` and continue inserting/stripping the empty delimiter manually — rejected because a `REP` peer does not speak the `DEALER`/`ROUTER` multi-frame envelope and expects strict lock-step. (b) Keep `DEALER` but send an explicit delimiter to a `REP` peer — rejected as fragile and non-idiomatic; `REQ` is the canonical counterpart to `REP`.
- **Rationale:** The architect confirmed the DB binds `REP`, validated against the Python3 SLC reference. `REQ`↔`REP` is the matching lock-step pattern and removes the hand-rolled framing that caused the post-`MT_CLIENT` hang.
- **Promotes to ADR:** yes

### [2] Replace the ROUTER mock peer in tests with a REP peer

- **Decision:** The transport tests mock the DB with a `zmq::REP` socket that simply recv-decodes one frame and encode-sends one frame, dropping all identity/delimiter envelope handling.
- **Alternatives:** Keep the `ROUTER` mock — rejected because it no longer reflects the real DB wire shape and would mask delimiter regressions.
- **Rationale:** Tests must mirror the production peer (`REP`) so they prove the actual lock-step exchange rather than a `DEALER`/`ROUTER` simulation.
- **Promotes to ADR:** no

### [3] Keep the Background-text correction as a recorded migration, not a delta scenario

- **Decision:** Capture the `DEALER`/`ROUTER` → `REQ`/`REP` wording fix in the permanent feature Background via the plan's Migration section, applied at record time, rather than encoding it as a delta scenario.
- **Alternatives:** Author a full-feature delta to rewrite the Background — rejected as heavyweight for a wording correction with no behavioral scenario impact beyond the two changed scenarios.
- **Rationale:** Deltas for existing features carry scenario changes; Background prose is corrected during the record step.
- **Promotes to ADR:** no

## Review Findings

<!-- Populated by speq-implement after code review. -->
