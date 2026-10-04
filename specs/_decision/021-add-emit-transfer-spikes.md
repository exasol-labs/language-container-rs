# Decisions: add-emit-transfer-spikes

## ADR: Decision gate — promote Spike A (string-block fast-path), drop Spikes B and C

**ID:** decision-gate-promote-spike-a-drop-b-c
**Plan:** `add-emit-transfer-spikes`
**Status:** Accepted

### Context

Three feature-gated throwaway spikes were measured against the extended `benches/emit-bench` (live Exasol 2026.1.0 Docker, `wide` shape — `id BIGINT, amount DECIMAL(18,2), event_date DATE, event_ts TIMESTAMP, label VARCHAR(100)`, N=1,000,000, reduced-config single run) against the Arrow-IPC baseline (row 655,801 rows/s / 69.5 MB/s; batch 656,486 rows/s / 69.6 MB/s):

| Configuration | Mode | rows/s | MB/s | vs. baseline |
|---|---|---|---|---|
| Spike A — string-block fast-path | row | 959,326 | 101.7 | **+46%** |
| Spike A | batch | 839,368 | 89.0 | **+28%** |
| Spike B — Arrow C Data Interface | batch | 558,905 | 59.2 | **−15%** |
| Spike C — raw per-column buffers | batch | 609,038 | 64.6 | **−7%** |

This directly re-measures, with NUMERIC/DATE/TIMESTAMP data the fix-abi-feature-safety decision-log (2026-06-25) never covered, the exact question ADR-051 (#26) and ADR-052 (#31) previously settled.

### Decision

Promote Spike A to production quality: the hand-rolled fast formatter becomes the unconditional default for `value_to_block_string`'s Date/Timestamp/Decimal branches, plus the pre-sized `Vec::with_capacity` change in `to_proto`/`encode_slice`; the `spike-string-fast` feature gate is deleted entirely. Drop Spike B and Spike C: their code, Cargo features, tests, and the `UdfContext` methods/ext-traits they added are deleted, and `EXA_UDF_ABI_VERSION` is reverted 7 → 6 since removing both methods restores the trait to its pre-plan shape with no external contract at stake.

### Options Considered

| Option | Verdict |
|--------|---------|
| Promote Spike A, drop B and C | ✓ Chosen — clear, consistent positive signal (+28–46%) on both measured shapes; B and C underperformed the status quo |
| Investigate why Spikes B/C regressed before deciding | ✗ Rejected — user chose to act on the clear signal now rather than spend further time on two already-underperforming candidates |
| Re-run the full median-of-5, 1M/5M matrix before deciding | ✗ Rejected — the effect size (28–46% for A; consistent regressions for B/C) was judged unlikely to invert under more samples |

### Consequences

This new evidence **reinforces** rather than overturns the fix-abi-feature-safety ADR: Arrow IPC, a from-scratch Arrow C Data Interface, and a hand-rolled raw-buffer transport are all not the bottleneck for emit throughput even on string-block-heavy shapes — both alternative transports measured slower than the IPC baseline they were meant to beat. ADR-051 and ADR-052 remain the correct guardrails; the `TypeId`/vtable hazard class they protect against is not worth reopening for a transport mechanism that underperforms the status quo even when re-measured with better data. The dominant cost was, and remains, per-cell string-block formatting, now fixed by Spike A's promoted form. The ingest side was symmetrically productionised per the plan's Stage 5 (`decision-log[5]`, not independently promoted to an ADR since it is a scope/sequencing call, not a design decision).
