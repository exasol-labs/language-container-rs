# Decision Log: add-emit-transfer-spikes

Date: 2026-07-06

## Interview

**Q (direction):** Given the benchmark already shows Arrow IPC is only 2–9% of emit cost,
should the plan pivot away from the `.so`↔host FFI boundary and target per-cell proto
string-block encoding instead?
**A:** Do not pick a direction by argument. Do spikes on each of the three candidates:
come up with a strawman (most minimal) implementation and test the performance — take
data-based decisions and stop guessing. The goal is to be as fast as possible; it is
interesting that string-blocks are expensive to serialize, so pay close attention there
and see whether it can be optimised.

**Q (validation gate):** Should the plan include a benchmark-extension stage (add
decimal/date/timestamp-heavy shapes to `emit-bench`) to confirm this before optimising, or
go straight to implementing?
**A:** Extend the benchmark first (the recommended option). The existing benchmark's only
shape is `id BIGINT, label VARCHAR(100), val DOUBLE` — it has zero NUMERIC/DATE/TIMESTAMP
columns, so all three spikes must be measured against a schema that actually exercises the
string-block types under suspicion.

**Q (scope: emit vs. both directions):** Emit only, or both emit and ingest
(`decode_string_block` / `InputRowSet::from_proto`), since ingest has the symmetric
chrono-parsing cost?
**A:** "Revisit this with my answers provided" (deferred rather than picking directly).
Resolved by the orchestrator (see Design Decision [5]): include ingest as a sequenced
follow-on after the emit decision, reusing the winning technique symmetrically rather than
running a fresh 3-way spike. Flagged as an orchestrator judgment call the user can revisit
in review.

## Design Decisions

### [1] Resolve issue #29 by spike-and-measure, not by argument

- **Decision:** Build three minimal throwaway spikes (string-block encoding fast-path,
  Arrow C Data Interface, raw per-column buffers), benchmark each end-to-end, and let the
  numbers pick the winner.
- **Alternatives:** Pick a direction from first principles — either pivot to string-block
  optimisation (the "2–9%" argument) or re-adopt the reviewer's Arrow C Data Interface
  proposal outright.
- **Rationale:** The user explicitly rejected argument-based selection. A spike-and-measure
  gate turns a contested design question into an evidence question.
- **Promotes to ADR:** yes

### [2] Extend the benchmark (NUMERIC/DATE/TIMESTAMP shapes) before spiking

- **Decision:** Add NUMERIC/DECIMAL, DATE, and TIMESTAMP columns to `benches/emit-bench`
  (and an ingest read-back measurement) as an early task, before any spike is measured.
- **Alternatives:** Go straight to implementing against the existing single-shape
  benchmark.
- **Rationale:** The prior "2–9% of emit cost" finding was measured only against
  `id BIGINT, label VARCHAR(100), val DOUBLE` — zero string-block-heavy types. Optimising
  or deciding on that basis would still be guessing about the exact types
  (`chrono`/`Decimal` formatting) most likely to dominate.
- **Promotes to ADR:** yes

### [3] Spikes are throwaway, feature-gated, and off by default

- **Decision:** Each spike is minimal strawman quality, isolated behind its own feature,
  and not wired into the default build. Losing spikes are deleted, not left dormant.
- **Alternatives:** Build one production-quality candidate directly; or keep all spikes
  behind flags after the decision.
- **Rationale:** Strawman quality is sufficient to get a real number; keeping unpromoted
  FFI/raw-buffer code around would leave the #26/#31 `TypeId`/ABI hazard latent in the
  tree.
- **Promotes to ADR:** no

### [4] Re-measuring the previously-dropped Arrow C Data Interface is warranted, and is a re-measurement — not a silent reversal

- **Decision:** Spike B revisits the Arrow C Data Interface scope that the
  fix-abi-feature-safety decision-log (2026-06-25) dropped, ADR-051 (remove `query_arrow`,
  #26) and ADR-052 (feature-independent vtable, #31) constrain. The spike is explicitly
  framed as re-measuring with better data (new NUMERIC/DATE/TIMESTAMP shapes), stays
  feature-gated and off by default, and the decision-gate ADR must reference those prior
  decisions by identifier and state why re-measurement is justified even though it may
  reach the same conclusion.
- **Alternatives:** Treat the fix-abi-feature-safety ADR as final and refuse to
  re-evaluate; or silently re-adopt Arrow FFI without acknowledging the prior decision and
  the hazard class it protected against.
- **Rationale:** The prior decision was made on evidence that never covered the
  string-block types; new shapes are new evidence. But the `TypeId`/vtable hazard (two
  independently-linked static `arrow` copies) is real and cost #26/#31 to find — so
  reopening it must be deliberate, bounded, and documented, never a silent reversal.
- **Promotes to ADR:** yes

### [5] Ingest is a sequenced follow-on with symmetric technique reuse (orchestrator call)

- **Decision:** Include the ingest-side path (`decode_string_block`,
  `InputRowSet::from_proto`) in this same plan, sequenced after the emit-side decision, and
  apply whichever technique wins symmetrically — a fresh 3-way spike is NOT run for ingest.
  If the emit winner is Arrow FFI or raw buffers (both emit-specific per #29's framing),
  ingest has no mirror and the stage closes as N/A.
- **Alternatives:** Emit only (drop ingest); a full fresh 3-way ingest spike; a separate
  plan for ingest.
- **Rationale:** The user deferred this choice; the orchestrator resolved it to keep the
  symmetric optimisation in scope without doubling the spike work. Only the string-block
  encoding optimisation has a natural ingest mirror (fast parsing vs. fast formatting).
  This is an orchestrator judgment call, not a direct user choice — flagged here so it can
  be revisited in review.
- **Promotes to ADR:** no

### [6] The productionisation spec delta captures a behavioral invariant, not a chosen implementation

- **Decision:** The spec delta adds two implementation-agnostic scenarios asserting that
  any promoted fast-path encoder/decoder produces byte-identical wire output / round-trips
  identically to the current `chrono`/`Display` path, preserving `EMIT_BUFFER_LIMIT_BYTES`
  flush semantics and NULL / row-major layout. It does NOT name a chosen encoder.
- **Alternatives:** Write a spec delta describing the specific winning implementation
  (e.g. "use `itoa`/`ryu`"); or add no spec delta at all.
- **Rationale:** The winner is unknown at plan time, and the durable business contract is
  the wire format the DB parses (exact `%Y-%m-%d`, `%Y-%m-%d %H:%M:%S%.9f`, fixed-point
  decimal strings), not the encoder internals. This mirrors ADR-036 (specs must not encode
  implementation detail).
- **Promotes to ADR:** yes

### [7] Reuse `itoa`/`ryu` from the lockfile or hand-roll; avoid a new direct dependency

- **Decision:** Spike A evaluates whether `itoa`/`ryu` (already transitively in
  `Cargo.lock`) fit the required output formats, or whether a bespoke hand-rolled formatter
  is simpler for this exact shape. Prefer reusing a lockfile crate over adding a new direct
  dependency; do not reinvent a well-tested crate that already fits.
- **Alternatives:** Add a new formatting crate as a direct dependency; keep `chrono`'s
  generic formatting.
- **Rationale:** Minimal-footprint preference; the exact required formats are narrow and
  fixed, so a few lines may beat a dependency, but an existing lockfile crate that fits is
  better than reinvention.
- **Promotes to ADR:** no

### [8] Decision gate: promote Spike A (string-block fast-path), drop Spikes B and C

- **Measured results** (live Exasol 2026.1.0 Docker, `wide` shape —
  `id BIGINT, amount DECIMAL(18,2), event_date DATE, event_ts TIMESTAMP, label
  VARCHAR(100)` — N=1,000,000, RUNS=1 reduced config, all three spikes measured against
  the same baseline for comparability):

  | Configuration | Mode | rows/s | MB/s | vs. baseline |
  |---|---|---|---|---|
  | Baseline (current `main`, Arrow IPC) | row | 655,801 | 69.5 | — |
  | Baseline | batch | 656,486 | 69.6 | — |
  | Spike A — string-block fast-path | row | 959,326 | 101.7 | **+46%** |
  | Spike A | batch | 839,368 | 89.0 | **+28%** |
  | Spike B — Arrow C Data Interface | batch | 558,905 | 59.2 | **−15%** |
  | Spike C — raw per-column buffers | batch | 609,038 | 64.6 | **−7%** |

  (Spike A also measured on `mixed`: row 1,329,048 rows/s / 87.7 MB/s, batch 1,585,332
  rows/s / 104.6 MB/s — consistent, positive across both shapes.)

- **Decision:** Promote Spike A to production quality (Stage 4, task 5.1): make the
  hand-rolled fast formatter the unconditional default for `value_to_block_string`'s
  Date/Timestamp/Decimal branches and the pre-sized `Vec::with_capacity` change in
  `to_proto`/`encode_slice`; delete the `spike-string-fast` feature gate entirely (no
  reason to keep the slow path selectable once proven byte-identical and strictly
  faster). Drop Spike B and Spike C: delete their code, Cargo features, tests, and the
  `emit_record_batch_arrow_ffi`/`emit_raw_columnar` `UdfContext` methods and ext-traits
  they added. Since removing both methods restores the trait to its pre-plan shape,
  revert `EXA_UDF_ABI_VERSION` 7 → 6 rather than leaving a bump with nothing to show for
  it — the version number is only meaningful in this codebase, and this plan never got
  as far as shipping the intermediate 7-shaped trait, so there is no external contract
  is at stake in retreating to 6.
- **This directly answers, with new data, the exact question the fix-abi-feature-safety
  ADR (2026-06-25) left open**: re-measuring against NUMERIC/DATE/TIMESTAMP shapes it
  never covered. The new evidence **reinforces** that ADR's conclusion rather than
  overturning it — Arrow IPC ser/deser (and, this plan additionally shows, a from-scratch
  Arrow C Data Interface or hand-rolled raw-buffer transport) is not the bottleneck for
  emit throughput even on string-block-heavy shapes; both alternative transports measured
  *slower* than the IPC baseline they were meant to beat, plausibly because per-call
  marshalling/struct-export overhead in a minimal spike outweighs whatever copy it
  avoids at this batch granularity. ADR-051 (#26) and ADR-052 (#31) remain the correct
  guardrails: the `TypeId`/vtable hazard class they protect against is not worth
  reopening for a transport mechanism that, even when re-measured with better data,
  performs worse than the status quo. The dominant cost was — and remains — per-cell
  string-block formatting (`chrono`'s generic `.format()`, `Decimal`'s generic
  `Display`), exactly as Spike A's result demonstrates directly.
- **Alternatives considered:** Investigate why Spikes B/C regressed before deciding
  (rejected — the user chose to act on the clear signal now rather than spend further
  time on two candidates that already underperform the status quo); re-run the full
  median-of-5, 1M/5M matrix before deciding (rejected — the user judged the reduced-config
  signal clear enough, given the magnitude of the effect (28-46% for A, consistent
  regressions for B/C) is unlikely to invert under more samples).
- **Promotes to ADR:** yes

## Review Findings

<!-- Populated by speq-implement after code review. -->
