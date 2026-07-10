# Decisions: add-emit-transfer-spikes

## ADR: Resolve issue #29 by spike-and-measure, not by argument

**ID:** resolve-issue-29-spike-and-measure
**Plan:** `add-emit-transfer-spikes`
**Status:** Accepted

### Context

GitHub issue #29 proposed a bespoke columnar wire format to speed up the `.so`↔host `emit_batch` hop; a reviewer counter-proposed the Arrow C Data Interface instead. An existing benchmark had already found Arrow IPC ser/deser to be only 2–9% of `emit_batch`'s total cost, suggesting per-cell proto string-block encoding (NUMERIC/DATE/TIMESTAMP formatting) was the more likely bottleneck — but that benchmark never exercised those types, so the direction was contested and evidence-thin either way.

### Decision

Build three minimal throwaway spikes — string-block encoding fast-path, Arrow C Data Interface, and raw per-column buffers — benchmark each end-to-end against the same extended shapes, and let the numbers pick the winner rather than deciding from first principles.

### Options Considered

| Option | Verdict |
|--------|---------|
| Spike-and-measure all three candidates | ✓ Chosen — turns a contested design question into an evidence question |
| Pivot to string-block optimisation by argument (the "2–9%" figure) | ✗ Rejected — the figure was never measured against NUMERIC/DATE/TIMESTAMP types |
| Re-adopt the Arrow C Data Interface outright by argument | ✗ Rejected — no data supported this either |

### Consequences

Three feature-gated, off-by-default spikes were built and measured before any production code changed. The decision gate (ADR-062) recorded the measured results and the promote/drop outcome per spike.

## ADR: Extend the benchmark (NUMERIC/DATE/TIMESTAMP shapes) before spiking

**ID:** extend-benchmark-numeric-date-timestamp-shapes
**Plan:** `add-emit-transfer-spikes`
**Status:** Accepted

### Context

The existing `benches/emit-bench` benchmark's only shape was `id BIGINT, label VARCHAR(100), val DOUBLE` — zero NUMERIC/DATE/TIMESTAMP columns. The prior "2–9% of emit cost" finding for Arrow IPC was therefore never measured against the string-block-heavy types under suspicion, so any spike measured against the old shape matrix would still be a guess about which types dominate.

### Decision

Extend `benches/emit-bench`'s shape matrix to add NUMERIC/DECIMAL, DATE, and TIMESTAMP columns, and add an ingest/read-back transfer measurement, before measuring any of the three spikes.

### Options Considered

| Option | Verdict |
|--------|---------|
| Extend the benchmark first (validation gate) | ✓ Chosen — spikes are measured against schemas that actually exercise the types under suspicion |
| Go straight to implementing against the existing single-shape benchmark | ✗ Rejected — would leave the exact same evidence gap that made the original question contested |

### Consequences

`benches/emit-bench` gained NUMERIC/DATE/TIMESTAMP shapes and an ingest read-back measurement ahead of any spike work, giving all three spikes (and the eventual production change) a shared, representative baseline.

## ADR: Re-measuring the previously-dropped Arrow C Data Interface is warranted, and is a re-measurement — not a silent reversal

**ID:** re-measure-arrow-c-data-interface-not-a-reversal
**Plan:** `add-emit-transfer-spikes`
**Status:** Accepted

### Context

The fix-abi-feature-safety decision-log (2026-06-25), ADR-051 (`query_arrow` removal, #26), and ADR-052 (feature-independent vtable, #31) previously dropped the Arrow C Data Interface as a `.so`↔host transport, on evidence that never covered NUMERIC/DATE/TIMESTAMP shapes. New shape data is new evidence, but the `TypeId`/vtable hazard (two independently-linked static `arrow` copies) that motivated those prior decisions is real, so reopening the question needed to be deliberate and bounded rather than an implicit do-over.

### Decision

Spike B re-measures the Arrow C Data Interface against the new NUMERIC/DATE/TIMESTAMP shapes, explicitly framed as a re-measurement with better data, staying feature-gated and off by default, with the decision-gate ADR required to cite ADR-051/ADR-052 by identifier and state why re-measurement is justified even if it reaches the same conclusion.

### Options Considered

| Option | Verdict |
|--------|---------|
| Re-measure with new shape data, feature-gated, documented as a re-measurement | ✓ Chosen — treats new evidence as new evidence without silently reversing or reintroducing the ABI hazard by default |
| Treat the fix-abi-feature-safety ADR as final and refuse to re-evaluate | ✗ Rejected — the prior evidence gap was real and worth closing |
| Silently re-adopt Arrow FFI without acknowledging the prior decision | ✗ Rejected — would erase the hazard-class context ADR-051/ADR-052 recorded |

### Consequences

Spike B was built behind its own feature flag, measured, found to underperform the status quo (ADR-062), and deleted per that decision. ADR-051 and ADR-052 stand reinforced rather than overturned.

## ADR: The productionisation spec delta captures a behavioral invariant, not a chosen implementation

**ID:** productionisation-spec-delta-behavioral-invariant
**Plan:** `add-emit-transfer-spikes`
**Status:** Accepted

### Context

At plan time, which of the three spikes (if any) would win was unknown, so a spec delta naming a specific encoder/decoder implementation would either be wrong or need rewriting once the decision gate concluded. The durable business contract is the exact wire format the Exasol engine parses (`%Y-%m-%d` DATE, `%Y-%m-%d %H:%M:%S%.9f` TIMESTAMP, fixed-point decimal), not which Rust code produces it.

### Decision

Add two implementation-agnostic scenarios to `runtime/dispatch-run-loop`: any promoted emit fast-path encoder must stay byte-identical to the current `chrono`/`Display` row path, and any promoted ingest fast-path decoder must round-trip byte-identically — both preserving `EMIT_BUFFER_LIMIT_BYTES` flush semantics and NULL/row-major layout. Neither scenario names a chosen encoder.

### Options Considered

| Option | Verdict |
|--------|---------|
| Implementation-agnostic byte-identical/round-trip invariant | ✓ Chosen — durable regardless of which spike wins; mirrors ADR-036 |
| Spec delta naming the specific winning implementation | ✗ Rejected — winner unknown at plan time; would encode implementation detail into the spec |
| No spec delta at all | ✗ Rejected — the byte-identical wire-format contract is a real, durable business requirement worth specifying |

### Consequences

`runtime/dispatch-run-loop/spec.md` gained two scenarios asserting the byte-identical/round-trip invariant, satisfied first by Spike A and then unconditionally by its promoted, production-quality form.

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
