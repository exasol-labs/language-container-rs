# Plan Review Findings: refactor-rowset-dispatch-complexity (round 2)

## Summary
- Axes checked: 6/6
- Total findings: 7 (Blockers: 0, Advisory: 7 — 5 carried unaddressed from round 1, 2 new)
- Intent Fidelity blockers: 0

## Round-1 Blocker Recheck

- Resolved: [UNSTATED_ASSUMPTION] coverage "measurement defect" premise — plan.md Context ¶ "A fourth, separate finding" now states the verified cause (feature unification from `exaudfclient`'s `features = ["connect-back"]`; gated code compiled and measured; SonarCloud arm-level gaps 647–650/676–696/707–720 cited). Decision-log [1] rewritten with the falsification recorded verbatim; its ADR promotion cancelled and re-seated on the corrected CI-hardening decision [8]. Phase 2 re-titled "no measurement change"; task 2.1 correctly states the gated suites already run and re-justifies `--all-features` as pinning an otherwise-inherited feature set. Phase 5 split into measure (5.1, `cargo llvm-cov -p exa-udf-runtime -p exaudfclient` — correctly reproduces CI's unified set) plus per-file closure tasks sized to the real 315/54/40 baselines (5.2/5.3), with the DB-bound `open_connect_back` carve-out recorded in decision [1], task 5.2, and Manual Testing row 4. `[plan-review]` entry [1] present. Not reworded — re-analyzed.
- Resolved: [UNSTATED_ASSUMPTION] rowset duplication misattribution — Context item 2 now names the true Sonar blocks (`impl UdfContext` bodies, 1683↔1908 × 57 and 1819↔1982 × 35, matching the duplications API exactly); item 3 explicitly demotes the encoder pair to conceptual leakage ("Sonar does not flag this pair"). Decision [3] rationale re-scoped. New task 3.4 + decision [9] give the flagged blocks a `macro_rules!` owner; verified sound against the code: the block boundaries start after the impls' differing `num_columns`, both structs carry `conn_requester`/`record_error` so one macro body serves both, and CPD dedup via macro invocation is textually effective. All four named guard tests exist in `rowset_tests.rs` (including `host_bridge_debug_level_returns_valid_level`, `single_call_context_debug_level_returns_valid_level` — verified by grep). Verification § now shows the arithmetic (254/6,324 = 4.0% → ≈ 2.9% via wire.rs → ≈ 0 from this scope via 3.4); the "well under 2%" quote checks out against the live issue #60 body (`gh issue view 60`). Dead-code table row added. `[plan-review]` entry [2] present.

## Intent Fidelity

no objection — axis checked: revision introduces no drift. Task 3.4 traces directly to issue #60's scope line "184 duplicated lines / 4 blocks" in rowset.rs. The DB-bound coverage carve-out is recorded openly in decision [1] and task 5.2 (not silently dropped) and fits the interview's "essentially all" phrasing; the "well under 2%" duplication expectation quoted in Verification is genuine issue text, and the plan's arithmetic exceeds it.

## Feasibility

no objection to the new material — axis checked: task 3.4's macro approach compiles-in-principle against the real struct shapes (both impls' duplicated bodies are token-identical, per-method `#[cfg]` moves inside the macro, differing `get`/`emit`/`next`/`set_return`/`num_columns` stay hand-written); task 5.1's measure command reproduces CI's unified feature set; task 5.2's `request_connection` fake-`ConnRequester` route exists (`rowset.rs:1652`). Two round-1 advisories on this axis remain unaddressed:

#### [EFFORT_MISESTIMATION] ADVISORY (carried from round 1, unaddressed)
- Location: plan.md § Phase 4 task 4.1
- Issue: The `conn_requester<'a>(&'a ZmqTransport, &'a RefCell<&'a mut Protocol>)` sketch still conflates two lifetimes; `RefCell<&mut _>` invariance makes it uncompilable as written, and `invoke_vs_adapter_call` must move its `RefCell` construction out to the caller. Untagged task, hidden work.
- Fix: As round 1: two lifetime parameters (e.g. `<'a, 'p: 'a>` with `&'a RefCell<&'p mut Protocol>`) and a sub-bullet moving the `RefCell` construction in `single_call.rs` to the call site.

#### [NFR_IGNORED] ADVISORY (carried from round 1, unaddressed)
- Location: plan.md § Phase 3 task 3.2; decision-log.md § [3]
- Issue: `Utf8`/`LargeUtf8` still routed through `value_to_block_string(&accessor_value(…))` in `encode_slice` — doubles per-cell string allocations on the emit_batch hot path for arms that carry none of the leaked epoch/unit/decimal knowledge.
- Fix: As round 1: exempt `Utf8`/`LargeUtf8` from the detour (keep direct `to_string()` pushes), or add an emit-bench before/after run to § Verification.

## Requirement Quality

no objection to the new material — axis checked: task 3.4 is testable as written (named guards, clippy gate); tasks 5.1–5.3 have a concrete oracle (`--show-missing-lines` empty except documented DB-bound lines); no conflict with the four recorded specs. Two round-1 advisories remain unaddressed:

#### [AMBIGUOUS_REQUIREMENT] ADVISORY (carried from round 1, unaddressed)
- Location: plan.md § Phase 4 tasks 4.1/4.3; decision-log.md § [4]
- Issue: `wire::conn_requester`'s ping policy is still unspecified — the existing inline closures use a raw send/recv/step exchange where a mid-MT_IMPORT ping is a hard error, unlike `wire::request`'s transparent retry; consolidation invites silently switching policies.
- Fix: As round 1: state in 4.1 that `conn_requester` keeps the raw exchange and MUST NOT route through `wire::request`; scope 4.3's ping test to the run/done/emit exchanges.

#### [COMPLETENESS_GAP] ADVISORY (carried from round 1, unaddressed)
- Location: plan.md § Phase 3 task 3.3
- Issue: `accessor_value` edge tests still name no pre-epoch (negative) inputs; the `TsNanosecond` `(ns % 1_000_000_000) as u32` wrap-to-default behavior remains unpinned and is the likeliest silent-change site during unification.
- Fix: As round 1: add negative `Date32` and negative-epoch cases for all four timestamp units, asserting current outputs byte-for-byte.

## Task Breakdown

no objection to the new material — axis checked: Group C's extension (3.1 → 3.2 → 3.3 → 3.4, same file) and Group E's split (5.1 → 5.2 ∥ 5.3, disjoint files) are sound; 5.2/5.3 baselines match the SonarCloud per-file numbers (315/54/40). One round-1 advisory remains unaddressed:

#### [TASK_GRANULARITY] ADVISORY (carried from round 1, unaddressed)
- Location: plan.md § Parallelization, Group F
- Issue: Group F still lists 6.1 and 6.2 unordered; the 6.2 version bump invalidates the ABI fingerprint of every locally built fixture `.so`, so a 6.1 run before (or parallel with) 6.2 certifies a tree the bump then changes.
- Fix: As round 1: order Group F as 6.2 → 6.1 and restate the ordering in task 6.1.

## Design Depth

no objection — axis checked: decision [9] picks the lowest-blast-radius owner for the duplicated delegation bodies and correctly rejects the SDK-trait-default alternative as an out-of-scope API change; the macro keeps each impl's divergent policies hand-written (no forced unification of deliberately different behavior, consistent with decision [5]); decision [8] elevates two real invariants (explicit fixture list, declared measurement configuration) that mirror the existing CLAUDE.md CI rule — a defensible ADR.

## Prose Quality

#### [PROSE_UNCLEAR] ADVISORY (new)
- Location: plan.md § Design › Context, ¶ opening "A fourth, separate finding"
- Issue: The list above it now enumerates four code smells ("Four code smells drive this"), so the coverage paragraph is a fifth, separate item; "fourth" is a leftover from the three-item draft and miscounts on one read.
- Fix: Change the paragraph opener to "A separate finding:" (drop the ordinal).

#### [PROSE_UNCLEAR] ADVISORY (new)
- Location: plan.md § Design › Non-Goals
- Issue: "no coverage push for `connect_back.rs` beyond what the measurement fix reveals" — the revision removed the "measurement fix" concept (Phase 2 is now explicitly "no measurement change"), leaving this Non-Goal referencing a mechanism the plan no longer contains.
- Fix: Reword to "no coverage push for `connect_back.rs` (out of issue scope)".
