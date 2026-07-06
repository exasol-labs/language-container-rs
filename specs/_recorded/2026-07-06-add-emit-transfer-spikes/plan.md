# Plan: add-emit-transfer-spikes

## Summary

Resolve the UDF↔DB data-transfer performance question (GitHub issue #29) with data
instead of argument: extend the emit benchmark to exercise the NUMERIC/DATE/TIMESTAMP
string-block types under suspicion, build three minimal throwaway spikes (string-block
encoding fast-path, Arrow C Data Interface, raw per-column buffers), measure each, then
promote only the spike(s) that show a measurable win — preserving the exact wire-format
contract — and drop the rest with a documented rationale.

## Design

### Context

`emit_batch` moves rows across two hops: the `.so`→SLC hop (UDF cdylib to host runtime)
and the SLC→DB ZMQ hop (`MT_EMIT` protobuf frames). Issue #29 proposes a bespoke
columnar wire format to speed up the first hop; a reviewer counter-proposed the Arrow C
Data Interface. An existing end-to-end benchmark (`benches/emit-bench`) already found
that Arrow IPC ser/deser is only 2–9% of `emit_batch`'s total cost — the dominant cost is
per-cell proto string-block encoding (stringifying NUMERIC/DATE/TIMESTAMP/VARCHAR cells
in `value_to_block_string` and the reverse in `decode_string_block`, in
`crates/exa-udf-runtime/src/rowset.rs`). On that evidence the team already dropped the
Arrow C Data Interface scope (fix-abi-feature-safety decision-log, 2026-06-25), removed
`ExaConnection::query_arrow` (ADR-051, #26), and hardened the vtable ABI (ADR-052, #31).

The crucial caveat: that benchmark's only shape is `id BIGINT, label VARCHAR(100), val
DOUBLE` — it has **zero** NUMERIC/DATE/TIMESTAMP columns, so the "2–9%" figure was never
measured against the very types whose formatting path (`chrono`'s generic `format` /
`parse_from_str`, plus `Decimal`'s custom `Display`) is plausibly the most expensive part
of string-block encoding. The plan therefore treats this as a **spike-and-measure**
decision, not a foregone conclusion in either direction.

- **Goals** — Make emit (and, as a sequenced follow-on, ingest) of NUMERIC/DATE/TIMESTAMP-
  heavy data as fast as possible; pick the winner from benchmark numbers; keep the
  SLC→DB wire encoding byte-identical so downstream Exasol parsing is unaffected.
- **Non-Goals** — Productionising more than one approach; changing the `MT_EMIT` wire
  limit (`EMIT_BUFFER_LIMIT_BYTES = 4_000_000`, a hard DB cap); altering the row-major
  block layout or NULL-bitmap semantics; re-opening the `TypeId`/ABI hazard class of
  #26/#31 in the default build (any FFI/raw-buffer spike stays feature-gated and off by
  default unless the decision gate explicitly promotes and re-hardens it).

### Decision

Extend the benchmark first (validation gate), then spike three candidates in parallel,
each measured against the extended benchmark, then a decision gate promotes the winner(s)
and drops the losers. Spikes are throwaway strawman quality: minimal, feature-gated, not
wired into the default build. The productionised behaviour is captured as an
implementation-agnostic invariant (byte-identical wire output), not as a chosen strategy.

#### Architecture

```
                       ┌───────────────────────────┐
 benches/emit-bench    │  extend shape matrix       │  (dev harness, no spec delta)
 + NUMERIC/DATE/TS  ──▶│  + ingest read-back timing │
                       └────────────┬──────────────┘
                                    │ baseline numbers
              ┌─────────────────────┼─────────────────────┐
              ▼                     ▼                     ▼
      Spike A: string-      Spike B: Arrow C       Spike C: raw per-
      block fast-path       Data Interface         column buffers
      (no FFI/ABI)          (arrow::ffi) [expert]  (&[u8]) [expert]
              └─────────────────────┼─────────────────────┘
                                    ▼
                       ┌───────────────────────────┐
                       │  Decision gate (ADR)       │  cites ADR-051, ADR-052,
                       │  keep winner / drop rest   │  fix-abi-feature-safety
                       └────────────┬──────────────┘
                                    ▼
                    productionise winner (behavioral spec delta)
                                    ▼
                    ingest follow-on (symmetric reuse, string-block only)
```

#### Patterns

| Pattern | Where | Why |
|---------|-------|-----|
| Spike-and-measure | all three candidates | Replace first-principles argument with numbers (interview Q1) |
| Measure-before-optimise | benchmark extension precedes spikes | The "2–9%" figure was never measured against the string-block types; validate first (interview Q2) |
| Feature-gated throwaway | Spike B, Spike C | Keep the `TypeId`/ABI hazard (#26/#31) out of the default build until re-hardened |
| Implementation-agnostic invariant | productionisation spec delta | Spec encodes the wire-format contract, not the encoder impl (mirrors ADR-036) |
| Symmetric reuse (no fresh spike) | ingest follow-on | FFI/raw-buffer are emit-specific per #29; only string-block has an ingest mirror |

### Consequences

| Decision | Alternatives Considered | Rationale |
|----------|------------------------|-----------|
| Three parallel throwaway spikes, then decide | Pick a direction by argument (pivot to string-block, or re-adopt Arrow FFI outright) | User rejected argument-based decision; stop guessing, take data-based decisions |
| Extend benchmark before spiking | Go straight to implementing | The prior "2–9%" evidence never covered NUMERIC/DATE/TIMESTAMP; without those shapes any decision is still a guess |
| Re-measure previously-dropped Arrow C Data Interface | Treat fix-abi-feature-safety ADR as final | New shape data warrants re-measurement; the spike is explicitly framed as re-measuring, not silently reversing — it stays feature-gated so #26/#31 hazard is not reintroduced into the default build |
| Behavioral (byte-identical) spec delta only | Write a spec delta for the chosen encoder | Winner is unknown at plan time; the durable contract is the wire format, not the implementation |
| Ingest as sequenced follow-on, symmetric reuse | Fresh 3-way spike for ingest; or drop ingest entirely; or separate plan | Orchestrator judgment call (see decision-log [5]); ingest mirrors only the string-block optimisation — flagged for user review |

## Features

| Feature | Status | Spec |
|---------|--------|------|
| runtime/dispatch-run-loop | CHANGED | `runtime/dispatch-run-loop/spec.md` |

The benchmark extension (Stage 0) and the three spikes (Stage 2) are dev/test-harness and
throwaway-exploration mechanics — per project rules they carry **no** spec delta. Only the
promoted, productionised behaviour touches the spec library, and only as the
implementation-agnostic byte-identical-wire-output invariant added above.

## Dependencies

- `itoa` / `ryu` are already present transitively in `Cargo.lock`. Spike A evaluates
  whether they fit the required output formats (fixed-point decimal, `%Y-%m-%d`,
  `%Y-%m-%d %H:%M:%S%.9f`) or whether a bespoke hand-rolled formatter is simpler for this
  exact shape. Prefer reusing a lockfile crate over adding a new direct dependency; do not
  reinvent a well-tested crate that already fits.
- `arrow` stays pinned to the `exarrow-rs` version (workspace `Cargo.toml`); Spike B uses
  `arrow::ffi` (ArrowArray/ArrowSchema) only, behind its own feature.

## Implementation Tasks

Stage 0 — Benchmark extension (validation gate, dev harness):

1.1 Extend `benches/emit-bench` shape matrix to add NUMERIC/DECIMAL, DATE, and TIMESTAMP
    columns alongside the existing `id BIGINT, label VARCHAR(100), val DOUBLE` shape; wire
    the new shape into the bench UDF (`benches/emit-bench-udf`) and document it in
    `benches/README.md`.
1.2 Add an ingest / read-back transfer measurement to `emit-bench` (decode-side cost:
    `InputRowSet::from_proto` / `decode_string_block`) using the same shapes, if the
    harness does not already measure it.

Stage 1 — Baseline:

2.1 Run the extended benchmark on current `main` to establish per-shape baseline numbers
    (emit and ingest); record whether the "2–9%" finding holds for the NUMERIC/DATE/
    TIMESTAMP shapes or whether string-block formatting dominates. Captured as input to
    the decision-gate ADR.

Stage 2 — Three throwaway spikes (parallel; each minimal, feature-gated, off by default):

3.1 Spike A — string-block encoding fast-path: hand-rolled / `itoa`/`ryu` formatters for
    Decimal→string and Date/Timestamp→string (replacing `chrono`'s generic `format` and
    `Decimal`'s `Display`), plus pre-sized `Vec::with_capacity` for the proto string/other
    blocks in `to_proto` (currently `Vec::new()`). No FFI/ABI surface touched. Measure
    against the extended benchmark.
3.2 Spike B — Arrow C Data Interface: minimal `arrow::ffi` (ArrowArray/ArrowSchema) path
    passing buffers by pointer across the `.so` boundary instead of Arrow IPC
    serialize/deserialize for `emit_batch`. Isolated behind its own feature, NOT wired into
    the default build. Measure against the extended benchmark. [expert]
3.3 Spike C — raw per-column buffers (#29's literal proposal): hand-rolled non-Arrow
    columnar buffer (value buffer + validity bitmap + offsets for variable-width + a small
    type tag) as `&[u8]` across the `.so` boundary. Must round-trip every `ExaType`
    including NULL handling and the row-major-interleaved layout. Feature-gated. Measure
    against the extended benchmark. [expert]

Stage 3 — Decision gate:

4.1 Author the decision-gate ADR in this plan's `decision-log.md`: record the three spike
    results (numbers from running Stage 2, not pre-declared), the comparison methodology,
    and the criteria for promoting a spike to production quality vs. discarding it. MUST
    cite ADR-051, ADR-052, and the fix-abi-feature-safety decision-log (2026-06-25) it
    revisits, and MUST explain why re-measuring is warranted now (new NUMERIC/DATE/
    TIMESTAMP shapes) even if it reaches the same conclusion. Decide keep/drop per spike.

Stage 4 — Productionisation (gated on 4.1 outcome):

5.1 Productionise the promoted emit-side spike(s) to production quality and delete the
    losing spikes' strawman code/features. Uphold the byte-identical-wire-output invariant
    (spec scenario "A promoted emit fast-path encoder stays byte-identical to the row
    path"): identical proto output to the current `chrono`/`Display` row path for every
    representable value, preserved `EMIT_BUFFER_LIMIT_BYTES` flush semantics, preserved
    NULL / row-major-interleaved layout. Add unit round-trip tests and an integration
    db-roundtrip test with NUMERIC/DATE/TIMESTAMP columns. [expert]

Stage 5 — Ingest follow-on (sequenced after the emit decision; orchestrator scope call):

6.1 If (and only if) the string-block encoding spike won, apply the same technique
    symmetrically to the ingest decode path (fast parsing in `decode_string_block` /
    `InputRowSet::from_proto`) — no fresh 3-way spike (FFI/raw-buffer are emit-specific per
    #29). Uphold the invariant "A promoted ingest fast-path decoder round-trips
    byte-identically". Add unit round-trip + integration decode tests. If the winner was
    Arrow FFI or raw buffers, record that ingest has no symmetric mirror and close this
    stage as N/A in the decision-log.

Stage 6 — Close-out (recorder/implement stage; do NOT close during planning):

7.1 Comment on GitHub issue #29 referencing the new decision-gate ADR (keep or close
    per the decision), mirroring how the fix-abi-feature-safety ADR resolved the Arrow-FFI
    question. Do not close #29 during planning — this is an implementer/recorder task.
7.2 Bump `[workspace.package].version` (SemVer) and the pinned `exasol-udf-sdk` entry in
    `[workspace.dependencies]`; commit the regenerated `Cargo.lock` in the same change
    (per project rules).

## Parallelization

| Parallel Group | Tasks |
|----------------|-------|
| Group A | 1.1, 1.2 |
| Group B | 3.1, 3.2, 3.3 |

Sequential dependencies:
- Group A → 2.1 (baseline needs the extended shapes)
- 2.1 → Group B (spikes measured against the extended benchmark)
- Group B → 4.1 (decision gate needs all three spike results)
- 4.1 → 5.1 (productionise the chosen winner)
- 5.1 → 6.1 (ingest follow-on reuses the promoted emit technique)
- 4.1 → 7.1 (close-out references the ADR); 5.1/6.1 → 7.2 (version bump on the shipped change)

## Dead Code Removal

| Type | Location | Reason |
|------|----------|--------|
| Feature + module | losing spikes (Stage 2) | Throwaway strawman code for the approaches the decision gate does not promote MUST be deleted, not left dormant behind a feature flag |
| Formatter | `value_to_block_string` / `Decimal` `Display` old path | Removed only if Spike A wins and replaces it; the byte-identical invariant must hold before removal |

## Verification

### Scenario Coverage

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| A promoted emit fast-path encoder stays byte-identical to the row path | Unit | `crates/exa-udf-runtime/src/rowset.rs` (`#[cfg(test)]`) | `fast_path_to_proto_byte_identical_to_row_path` |
| A promoted emit fast-path encoder stays byte-identical to the row path | Integration | `crates/it/tests/db_roundtrip.rs` | `numeric_date_timestamp_emit_roundtrips` |
| A promoted ingest fast-path decoder round-trips byte-identically | Unit | `crates/exa-udf-runtime/src/rowset.rs` (`#[cfg(test)]`) | `fast_path_from_proto_matches_chrono_decode` |
| A promoted ingest fast-path decoder round-trips byte-identically | Integration | `crates/it/tests/db_roundtrip.rs` | `numeric_date_timestamp_ingest_roundtrips` |

Byte-identical / round-trip encoding is pure computation with no I/O, so the primary proof
is a unit test comparing the fast path against the current `chrono`/`Display` path over the
full `ExaType` range (incl. NULL and shared-block-type columns); the integration test
confirms the same bytes survive an end-to-end DB round-trip.

### Manual Testing

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| runtime/dispatch-run-loop (benchmark) | `cargo build --release -p emit-bench-udf && cargo run --release -p emit-bench` | Prints the throughput table including the NUMERIC/DATE/TIMESTAMP shape rows for {row, columnar} × {Rust, Python3} × {1M, 5M}; the string-block-heavy shape's emit and ingest transfer numbers are populated |
| runtime/dispatch-run-loop (round-trip) | `cargo test -p it --features integration numeric_date_timestamp` | Integration NUMERIC/DATE/TIMESTAMP emit + ingest round-trip tests pass against the local Exasol Docker container (fail, not skip, if the DB is unavailable) |

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Build | `cargo build --release -p exaudfclient` | Exit 0 |
| Build all | `cargo build --release` | Exit 0 |
| Unit test | `cargo test` | 0 failures |
| Integration | `cargo test -p it --features integration` | 0 failures (fail, not skip, if Docker DB unavailable) |
| Lint | `cargo clippy --all-targets --all-features -- -D warnings` | 0 warnings |
| Format | `cargo fmt --check` | No changes |
