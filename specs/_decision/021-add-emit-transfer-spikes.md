# Decisions: add-emit-transfer-spikes

## ADR: Decision gate: promote Spike A (string-block fast-path), drop Spikes B and C

**ID:** decision-gate-promote-spike-a-drop-b-c
**Plan:** `add-emit-transfer-spikes`
**Status:** Accepted

### Context

Per-cell string-block formatting dominates emit cost. On the `emit-bench` `wide` shape, the string-block fast-path (Spike A) beat the Arrow IPC baseline by 28-46%. The Arrow C Data Interface (Spike B) and raw per-column buffers (Spike C) were slower than the baseline.

### Decision

The hand-rolled fast formatter is the unconditional default for Date, Timestamp and Decimal cells in the emit string block. The `spike-string-fast` feature gate does not exist. Spikes B and C, their Cargo features, tests and `UdfContext` additions are absent.

### Options Considered

| Option | Verdict |
|--------|---------|
| Promote Spike A, drop B and C | ✓ Chosen |
| Investigate why B and C were slower | ✗ Both underperform the baseline already |
| Re-run the full median-of-5 matrix | ✗ Effect sizes are unlikely to invert |

### Consequences

Arrow IPC stays the batch transport. Transports other than Arrow IPC are not worth the `TypeId`/vtable hazard.
