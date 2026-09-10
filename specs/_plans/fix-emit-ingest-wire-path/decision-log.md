# Decision Log: fix-emit-ingest-wire-path

## Interview

**Q:** Issue #95 has 11 findings. Which scope?
**A:** Everything (all 11).

**Q:** F2 (120s transport timeout): remove the cap entirely, or raise and log?
**A:** Remove entirely, rely on the engine watchdog.

**Q:** F4 (ingest copies): immediate fix only, or include the full InputRowSet storage redesign?
**A:** Include the full redesign.

## Design Decisions

### [1] Remove the transport wall-clock cap rather than raise it

- **Decision:** Delete `MAX_TOTAL_WAIT` and its elapsed-time branch. Retry `EAGAIN` indefinitely.
- **Alternatives:** Raise the cap and log. Rejected.
- **Rationale:** The engine's retry loop (`zmqinternal.cc:903-980`) aborts only on zombie/missing socket, never on time. Any finite cap turns a slow query into a VM crash. The database watchdog already ends wedged sessions.
- **Promotes to ADR:** yes

### [2] Include the full InputRowSet storage redesign

- **Decision:** Restructure `InputRowSet` to keep prost typed arrays as storage, materialise one row at a time into a reusable scratch `Vec<Value>`.
- **Alternatives:** Ship only the immediate fix (slice decode, by-value table, `into_iter` drain). Rejected by the user.
- **Rationale:** The immediate fix removes two copies but leaves one heap allocation per row.
- **Promotes to ADR:** no

### [3] `emit_owned` as a second method with a forwarding default

- **Decision:** Add `emit_owned(&mut self, values: Vec<Value>)` to `UdfContext` with a default that forwards to `emit`. Keep `emit`'s slice signature unchanged.
- **Alternatives:** Change `emit` to take `Vec<Value>` (breaks every UDF). Take `impl Into<Cow<[Value]>>` (not object-safe, can't cross `.so` boundary).
- **Rationale:** The forwarding default confines the new method: implementations that don't care never see it.
- **Promotes to ADR:** no

## Review Findings
