# Plan Review Findings: refactor-rowset-dispatch-complexity (round 1)

## Summary
- Axes checked: 6/6
- Total findings: 7 (Blockers: 2, Advisory: 5)
- Intent Fidelity blockers: 0

## Intent Fidelity

no objection — axis checked: every issue-#60 acceptance item maps to a task (S3776 rowset.rs:630 → 3.1; dispatch.rs:76 → 4.2; duplication → 4.1/3.2; invariant tests first → Phase 1 with normative ordering; both test suites → 6.1). Interview decisions are operationalized, not re-litigated: tests-only/no-spec-deltas honored (Features table all UNCHANGED, verified against `specs/runtime/*`); extraction shape decided by planner per mandate (decision [2]); the "close all 315+94 uncovered lines" commitment is retained in task 5.1 (its sizing is challenged under Feasibility, not its presence).

#### [UNSTATED_ASSUMPTION] BLOCKER
- Location: plan.md § Design › Context (¶ "A fourth, separate finding"), § Phase 2 task 2.1, § Manual Testing row 4; decision-log.md § [1]
- Issue: The plan's central coverage claim is false. It states the CI coverage run "uses default features, so every `#[cfg(feature = "emit-arrow")]` and `#[cfg(feature = "connect-back")]` line — including `compute_row_costs`, `encode_slice`, `arrow_batch_to_value_rows`, `push_batch`, and their existing `arrow_tests` — is invisible to Sonar. Most of the 315 'uncovered' lines in rowset.rs are tested code that the measurement never compiles." Verified otherwise, two ways. (a) Feature unification: `crates/exaudfclient/Cargo.toml` declares `exa-udf-runtime = { features = ["connect-back"] }`; in any multi-package invocation — including CI's `cargo llvm-cov --workspace --exclude it` — cargo unifies that onto exa-udf-runtime's own test build. Reproduced locally: `cargo test -p exa-udf-runtime -p exaudfclient --no-run` compiles the gated test targets `emit_arrow_dlopen` and `connect_back` with no feature flags. (b) SonarCloud per-line data on current `main`: `compute_row_costs` (rowset.rs:630) has lineHits=1; its uncovered lines are specific arms (647–650, 676–696, 707–720). The gated code is compiled, tested, and measured today; the 315 (rowset.rs) + 54 (dispatch.rs) + 40 (single_call.rs) uncovered lines are genuine gaps in compiled code. Consequences: task 2.1's `--all-features` is a measurement no-op (the unified set already equals all features for runtime+sdk); 2.1's sub-claim "`--all-features` newly runs `emit_arrow_dlopen`" is wrong (it already runs — today's job survives only via cache-restored fixtures); Phase 5's "residual coverage" is actually the bulk of the 409-line commitment; and decision [1] promotes the false claim to an ADR. The cited verification ("cargo test -p exa-udf-runtime --all-features passes locally") proves tests pass with features on — not that CI misses them.
- Fix: Rewrite decision-log [1] and the plan.md Context ¶4 to the verified cause: gated code is already compiled and measured via feature unification from exaudfclient; the uncovered lines are genuine test gaps. Cancel the ADR promotion (or re-promote the corrected fixture-hardening decision [8] instead). Re-scope Phase 2 as CI hardening only: keep the 8-fixture build list (needed today, since `emit_arrow_dlopen` already runs in the coverage job) and keep `--all-features` only if re-justified as making the accidental unification explicit. Replace task 5.1's single sweep with per-file test tasks sized against the real 315/54/40 gaps (start from `cargo llvm-cov -p exa-udf-runtime -p exaudfclient --show-missing-lines`), or record in the decision log a renegotiated coverage ambition. Update Manual Testing row 4's expected output to match.

#### [UNSTATED_ASSUMPTION] BLOCKER
- Location: plan.md § Design › Context item 2, § Phase 3 task 3.2; decision-log.md § [3]
- Issue: The rowset duplication is misattributed, so the plan's duplication remedy does not move the metric its acceptance gates on. Plan: "`encode_slice` and `arrow_batch_to_value_rows` each re-implement the Arrow-cell-to-SDK-`Value` conversion … — the bulk of the 184 duplicated lines in `rowset.rs`." SonarCloud's duplications API for rowset.rs shows exactly two pairs — lines 1683+57 ↔ 1908+57 and 1819+35 ↔ 1982+35 — the `impl UdfContext for HostContextBridge` vs `impl UdfContext for SingleCallContext` handshake-delegation and connect-back method bodies. Zero flagged lines fall in `encode_slice` (738–893) or `arrow_batch_to_value_rows` (904–987). Task 3.2 therefore reduces measured duplication by 0. The ≤3% acceptance then rests solely on wire.rs: 254 duplicated / 6324 lines = 4.0% today; removing the 70-line dispatch/single_call pair yields ≈184/6300 ≈ 2.9% — under the gate by roughly 5 duplicated lines, with no contribution from the plan's rowset work and the file the issue names (rowset.rs, "184 duplicated lines / 4 blocks") left fully duplicated.
- Fix: Correct plan.md Context item 2 and decision [3]'s rationale: `accessor_value` removes conceptual back-door leakage (Arrow epoch/unit knowledge), not Sonar-flagged duplication. Then either (a) add a Phase 3/4 task giving the duplicated `UdfContext` handshake-delegation bodies one owner (e.g. a `macro_rules!` delegation block or a `HandshakeMeta`-backed helper; planner's choice; preserve the `#[cfg(feature = "connect-back")]` method split and both impls' differing `get`/`emit`/`next` policies), with its own protection tests (`bridge_returns_handshake_metadata`, `single_call_context_returns_handshake_metadata`); or (b) show in plan.md § Verification the ≤3% arithmetic with wire.rs alone and record the ~5-line margin as an accepted risk in the decision log.

## Feasibility

#### [EFFORT_MISESTIMATION] ADVISORY
- Location: plan.md § Phase 4 task 4.1 (wire.rs signature sketch)
- Issue: `conn_requester<'a>(&'a ZmqTransport, &'a RefCell<&'a mut Protocol>) -> ConnRequester<'a>` cannot compile as sketched: `RefCell<&'a mut Protocol>` is invariant in `'a`, forcing the outer `&'a RefCell` borrow to equal the full `&mut Protocol` lifetime, which a run_group/invoke_vs_adapter_call-local `RefCell` cannot satisfy. Two lifetime parameters are required. Additionally `single_call.rs::invoke_vs_adapter_call` today moves its `RefCell` into the closure (single_call.rs:155–158); consuming `wire::conn_requester` forces the RefCell construction out to the caller. Small but real work hidden in an untagged (non-expert) task.
- Fix: In task 4.1, change the sketch to two lifetimes (e.g. `fn conn_requester<'a, 'p: 'a>(&'a ZmqTransport, &'a RefCell<&'p mut Protocol>) -> ConnRequester<'a>`) and add a sub-bullet: "invoke_vs_adapter_call constructs the `RefCell` locally and passes `&proto_cell` to `wire::conn_requester`."

#### [NFR_IGNORED] ADVISORY
- Location: plan.md § Phase 3 task 3.2; decision-log.md § [3]
- Issue: Routing `Utf8`/`LargeUtf8` through `value_to_block_string(&accessor_value(…))` doubles allocations on the emit_batch hot path: today `encode_slice` does one `arr.value(r).to_string()` push (rowset.rs:805–806); the detour builds `Value::String` (alloc 1) then `value_to_block_string`'s `s.clone()` (alloc 2, rowset.rs:1272) per non-null string cell. The plan's own rationale for keeping native blocks direct — "needless indirection for … arms that are not duplicated today" — applies equally to `Utf8`/`LargeUtf8`: their two-line arms carry no epoch/unit/decimal knowledge. The change is byte-identical but not cost-identical, on the path whose reason to exist is throughput (benches/emit-bench).
- Fix: In task 3.2 and the plan.md § Architecture tree, exempt `Utf8`/`LargeUtf8` from the `accessor_value` detour in `encode_slice` (keep the direct `to_string()` pushes alongside the native arms); route only `Date32`/`Ts*`/`Decimal128`/`NumericFrom*` through `value_to_block_string(&accessor_value(…))`. Alternatively require an emit-bench before/after run in § Verification.

## Requirement Quality

no objection on conflicts — axis checked: the plan ships no deltas; all 30 scenario titles in § Scenario Coverage match the recorded specs of the four bounding features verbatim (checked against `specs/runtime/{rowset-codec,emit-arrow-batch,dispatch-run-loop,dispatch-single-call}/spec.md`), and no task contradicts a recorded scenario.

#### [AMBIGUOUS_REQUIREMENT] ADVISORY
- Location: plan.md § Phase 4 task 4.1; decision-log.md § [4]
- Issue: `wire::conn_requester`'s ping policy is unspecified. Both existing inline closures use a raw `send`/`recv`/`step` exchange (dispatch.rs:108–126, single_call.rs:158–176): a DB ping mid-MT_IMPORT is a hard error ("MT_IMPORT reply was not ConnInfo"), unlike `request`'s transparent ping-retry. Consolidating all three helpers into one wire.rs invites "cleaning up" conn_requester to call `wire::request`, which would silently change behavior — the exact failure the behavior-preserving mandate forbids. Task 4.3's "ping-pong mid-exchange retry through `wire::request`" does not disambiguate which exchanges retry.
- Fix: Add to task 4.1: "`wire::conn_requester` keeps the raw send/recv/step exchange — it MUST NOT route through `wire::request`; a ping during MT_IMPORT remains an error, as today." Scope 4.3's ping test to the run/done/emit exchanges.

#### [COMPLETENESS_GAP] ADVISORY
- Location: plan.md § Phase 3 task 3.3
- Issue: The `accessor_value` edge tests name "the four timestamp units, `Date32` epoch offset" but no pre-epoch (negative) values. The behavior-preservation risk concentrates exactly there: `TsNanosecond` computes `(ns % 1_000_000_000) as u32`, which for negative `ns` wraps and collapses to `unwrap_or_default()` (epoch) — identically in both current copies (rowset.rs:841–848, 959–968). An implementer unifying into `accessor_value` may "fix" this to `div_euclid`/`rem_euclid`, silently changing emitted bytes for pre-1970 nanosecond timestamps, and no named parity test pins negative inputs.
- Fix: In task 3.3, add explicit cases: negative `Date32`, and negative-epoch values for all four timestamp units — asserting the current outputs (including the `TsNanosecond` wrap-to-default behavior) byte-for-byte.

## Task Breakdown

no objection on traceability — axis checked: every task traces to an issue item; every "(existing)" test in § Scenario Coverage was verified present by exact name (20 in `rowset_tests.rs`, 7 in `tests/dispatch.rs`, 4 in `tests/single_call.rs`, 1 in `tests/emit_arrow_dlopen.rs`, 5 in `crates/it/tests/db_roundtrip.rs`); both NEW tests trace to tasks 1.1/4.3; the 8-fixture list in 2.1 matches exactly the fixtures the runtime tests `dlopen` (`so_path(...)` + literal `lib*.so` references).

#### [TASK_GRANULARITY] ADVISORY
- Location: plan.md § Parallelization, Group F (6.1, 6.2)
- Issue: Group F lists 6.1 (full checklist incl. IT run) and 6.2 (version bump) as one unordered group, but 6.2 invalidates 6.1: the ABI fingerprint tracks the workspace version, so bumping after the test runs leaves every locally built `.so` stale — the exact fingerprint-mismatch failure task 6.2's own note warns about. Run in the listed order, 6.1's green result certifies a tree that 6.2 then changes.
- Fix: In § Parallelization, order Group F as 6.2 → 6.1 (bump, update pinned SDK entry, regenerate Cargo.lock, rebuild all test-udf `.so`s, then run the full checklist), and restate that ordering in task 6.1.

## Design Depth

no objection — axis checked: wire.rs gives the lockstep-exchange decision one `pub(crate)` owner and decision [4] correctly rejects both the sibling-import and protocol-crate placements (checked: the `dispatch → single_call::take_c_string` coupling exists at dispatch.rs:271; `exa-zmq-protocol` stays policy-free); `accessor_value` is genuine information-hiding for the Arrow epoch/unit decisions (the conversion bodies are copy-identical across rowset.rs:807–877 and 937–979, so one owner preserves both); decision [5] correctly keeps the deliberately divergent unexpected-event policies apart (verified in both loops); decision [6]'s extraction preserves the documented `RefCell`/`Cell` borrow discipline. The one structural leakage the design misses — the duplicated `UdfContext` handshake-delegation impls — is Blocker 2's subject and not double-counted here.

## Prose Quality

no objection — axis checked: plan.md leads with the conclusion, Summary respects the two-sentence cap, tasks start with verbs, claims are quantified (byte counts, line numbers, test names), and no filler or escape clauses found in governed prose. The factually wrong statements are handled as Feasibility blockers, not prose defects.
