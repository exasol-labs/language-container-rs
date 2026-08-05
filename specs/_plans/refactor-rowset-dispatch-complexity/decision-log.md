# Decision Log: refactor-rowset-dispatch-complexity

## Interview

**Q:** How ambitious should the new unit-test coverage be for rowset.rs (315 uncovered lines today)?
**A:** Close all uncovered lines — aim to cover essentially all 315 uncovered lines in rowset.rs plus the 94 (54 + 40) in dispatch.rs/single_call.rs.

**Q:** Should the emit invariants also become permanent spec scenarios where missing?
**A:** Tests only, no spec changes — pure refactor; pin invariants as unit tests only; spec library untouched. Plan's spec deltas empty/minimal.

**Q:** For the compute_row_costs cognitive-complexity fix, which extraction shape?
**A:** Planner decides — pick between per-width-class helpers and a generic closure-based accumulator after reading the actual code; the issue allows either.

## Design Decisions

### [1] Coverage gaps are genuine test gaps; Phase 2 hardens the CI job without changing measurement

- **Decision:** Treat the 315 (`rowset.rs`) + 54 (`dispatch.rs`) + 40 (`single_call.rs`) uncovered lines as real gaps in compiled, measured code and close them with targeted tests (Phases 3–5). Phase 2 only hardens the coverage job: a complete fixture-build list and an explicit `--all-features` flag that pins the feature set the job already measures. Coverage ambition stays "close essentially all", with one recorded carve-out: lines requiring a live DB (`open_connect_back`'s exarrow session establishment) are out of unit reach and are listed explicitly in the verification report.
- **Alternatives:** The plan's first draft claimed the default-features CI build never compiles the gated code, making a measurement fix the primary remedy. Falsified two ways: `crates/exaudfclient/Cargo.toml` declares `exa-udf-runtime = { features = ["connect-back"] }`, which cargo unifies onto the runtime's test build in any workspace invocation; and SonarCloud per-line data on `main` shows `compute_row_costs` entered (lineHits=1) with specific arms uncovered.
- **Rationale:** The local `cargo test -p exa-udf-runtime --all-features` pass proved the full-matrix suite is green — not that CI misses it. With the misdiagnosis corrected, test-writing carries the coverage acceptance and the CI change is scoped to what it actually fixes: cold-cache fixture fragility and an implicit measurement configuration.
- **Promotes to ADR:** no

### [2] compute_row_costs: generic closure accumulator, not per-width-class helpers

- **Decision:** One helper `accumulate_costs(costs, nulls, cell_cost: impl Fn(usize) -> usize)` plus a `fixed_cell_cost(&DataType) -> Option<usize>` lookup; fixed-width arms pass a constant closure, `Utf8`/`LargeUtf8` pass a length closure.
- **Alternatives:** Two per-width-class helpers (fixed vs. variable) — two shallow modules where one deep one suffices; a declarative macro — hides control flow, saves no measured complexity, harder to read.
- **Rationale:** The duplicated decision is "skip NULL cells, add a per-cell cost"; the closure parameter is exactly the one thing that varies. Named width constants shared with `value_byte_cost` give the per-type byte-width table a single home.
- **Promotes to ADR:** no

### [3] accessor_value as the single Arrow-cell-to-Value conversion owner

- **Decision:** Extract `accessor_value(&ColAccessor, row) -> Value`; `arrow_batch_to_value_rows` pushes it directly, `encode_slice` routes string-block accessors through `value_to_block_string(&accessor_value(…))` and keeps direct native pushes for `Int32`/`Int64`/`Float64`/`Boolean`.
- **Alternatives:** Leave both functions as-is (keeps the back-door leakage: Arrow epoch offset and timestamp-unit conversions maintained in two places); route native blocks through `Value` too (needless indirection for the four copy-type arms).
- **Rationale:** One owner for the date/timestamp/decimal conversion knowledge, without touching the hot native-block path; byte-identity is enforced by the existing parity tests. This fixes conceptual leakage and complexity — it does not move the Sonar duplication metric, whose flagged `rowset.rs` blocks are the `UdfContext` impl bodies (see decision [9]).
- **Promotes to ADR:** no

### [4] New wire.rs module owns the lockstep DB exchange helpers

- **Decision:** Create `crates/exa-udf-runtime/src/wire.rs` holding `request` (ping-transparent REQ/REP exchange), `close_error`, and the `conn_requester` constructor; both dispatchers consume it.
- **Alternatives:** `single_call.rs` importing from `dispatch.rs` (makes the run-loop module an accidental owner of shared plumbing; the reverse coupling `dispatch → single_call::take_c_string` already exists and should not grow); moving the helpers into `exa-zmq-protocol` (wrong crate — `RuntimeError` and the ping-retry policy are runtime concerns, and the protocol crate must stay transport-policy-free).
- **Rationale:** Two modules independently assuming the same exchange convention is back-door leakage; the decision gets one home. `pub(crate)` keeps it invisible outside the runtime crate.
- **Promotes to ADR:** no

### [5] Dispatch and single-call loop semantics stay separate

- **Decision:** Do not unify `run_udf` and `run_single_call` over a shared loop; only the stateless helpers move to `wire.rs`. `single_call.rs::unexpected` stays where it is.
- **Alternatives:** A generic session loop parameterized by handler table — obvious-looking cleanup that changes behavior.
- **Rationale:** The two loops deliberately disagree on unexpected events: single-call hard-errors (retrying risks livelock), the run loop ignores and continues. A shared loop would force one policy on both.
- **Promotes to ADR:** no

### [6] run_group decomposition by extraction, preserving the borrow discipline

- **Decision:** Extract `emit_flusher`, `batch_fetcher`, `drive_group_rows`, and `tail_flush` as named functions; `run_group` becomes sequential composition. The `RefCell<&mut Protocol>` single-cell pattern and `Cell<Option<GroupExit>>` exit signaling are kept exactly as documented in the current comments.
- **Alternatives:** Restructure around a session-state struct owning transport + protocol (larger blast radius; changes the documented soundness argument for the non-overlapping borrows).
- **Rationale:** Sonar attributes closure nesting to the enclosing function; moving the closures behind named constructors removes the complexity attribution with zero behavior change and keeps the existing soundness reasoning valid.
- **Promotes to ADR:** no

### [7] Invariant pins added before restructuring; two gaps found

- **Decision:** Phase 1 adds only what is missing: a literal `EMIT_BUFFER_LIMIT_BYTES == 4_000_000` assertion (existing tests use the constant symbolically, so a 4 MiB regression would pass) and a row-path mid-run flush test at bridge level (only the batch path has one). The other invariants are already pinned by named existing tests, which the plan lists as the protection harness.
- **Alternatives:** Re-write all four invariants as new tests (duplicates healthy existing pins); skip the literal assertion (leaves the 4,000,000-vs-4-MiB confusion unpinned — the exact failure mode CLAUDE.md warns about).
- **Rationale:** Pin what a refactor could silently break; do not duplicate what already fails loudly.
- **Promotes to ADR:** no

### [8] CI coverage job hardened: complete fixture list, explicit feature set

- **Decision:** Task 2.1 extends the coverage job's "Build test fixture cdylibs" step to all eight fixtures the runtime tests `dlopen` (adding `set-sum`, `emit-k`, `scalar-next-illegal`, `returns-with-emit`, `emit-arrow-batch`) and adds `--all-features` to the `cargo llvm-cov` invocation.
- **Alternatives:** Leave the 3-fixture list (works only while the actions/cache restore happens to carry the `build` job's `target/`; on a cold cache the job fails today, because `emit_arrow_dlopen` and the other fixture-loading suites already run via feature unification and no step builds their `.so`s); omit `--all-features` (keeps the measured feature set implicit, inherited from `exaudfclient`'s `features = ["connect-back"]` declaration — a future change there would silently shrink measurement).
- **Rationale:** Every fixture a CI job's tests `dlopen` must be built by an explicit step of that job, not inherited from cache; and a measurement configuration must be declared, not accidental. Both mirror the existing CLAUDE.md rule that new `test-udfs/*` fixtures must be wired into CI's explicit `-p` allowlist.
- **Promotes to ADR:** yes

### [9] Sonar's rowset.rs duplication (the UdfContext impl bodies) gets a macro owner

- **Decision:** Task 3.4 defines `delegate_handshake_meta!` and `delegate_connect_back_hooks!` (`macro_rules!` in `rowset.rs`) and invokes them from both the `HostContextBridge` and `SingleCallContext` `UdfContext` impls, removing all 184 Sonar-flagged duplicated lines (1683↔1908 handshake getters, 1819↔1982 connect-back wrappers). The per-method `#[cfg(feature = "connect-back")]` split moves inside the macro; each impl's differing `get`/`emit`/`next`/`set_return` policies stay hand-written.
- **Alternatives:** Provided default methods on the `UdfContext` trait (an SDK API change — out of scope for a behavior-preserving runtime refactor); a shared inner context struct with `Deref` delegation (reshapes both impls, larger blast radius); accept the margin from `wire.rs` alone (≈ 2.9% project duplication, ~5 lines under the ≤ 3% gate, with the file the issue names left fully duplicated).
- **Rationale:** The issue's acceptance gates on the duplication metric, and the metric's `rowset.rs` findings are these two block pairs — not the encoder conversions. A textual macro owner removes them with zero behavior change, guarded by the existing handshake-metadata and debug-level tests for both context types.
- **Promotes to ADR:** no

## Review Findings

### [1] [plan-review] Coverage-gap misdiagnosis: feature unification already measures the gated code

- **Finding:** The plan claimed CI's default-features coverage build never compiles `emit-arrow`/`connect-back` code, making most of the 315 uncovered `rowset.rs` lines a measurement artifact. plan-reviewer falsified this: `exaudfclient`'s `features = ["connect-back"]` unifies onto `exa-udf-runtime`'s test build in the workspace invocation, and SonarCloud per-line data shows the gated code measured (e.g. `compute_row_costs` lineHits=1).
- **Direction change:** Context ¶4 and decision [1] rewritten to the verified cause (genuine test gaps in compiled code); decision [1]'s ADR promotion cancelled — the ADR promotion now sits on the corrected CI-hardening decision [8]; Phase 2 re-scoped to CI hardening (complete fixture list; `--all-features` re-justified as making the accidental unification explicit); Phase 5 replaced with a measure task (5.1, using `-p exa-udf-runtime -p exaudfclient` to reproduce CI's unified feature set) plus per-file closure tasks sized against the real 315/54/40 baselines (5.2, 5.3), with DB-bound lines carved out and documented; Manual Testing row 4 updated.
- **Promotes to ADR:** no

### [2] [plan-review] rowset.rs duplication misattributed to the encoder conversions

- **Finding:** The plan attributed the 184 Sonar-flagged duplicated `rowset.rs` lines to `encode_slice`/`arrow_batch_to_value_rows`; SonarCloud's duplications data shows they are the `UdfContext` impl bodies (1683↔1908, 1819↔1982), so task 3.2 moves the duplication metric by zero and the ≤ 3% gate would have rested on a ~5-line margin from `wire.rs` alone.
- **Direction change:** Context corrected (flagged duplication vs. conceptual leakage now distinct items); decision [3]'s rationale re-scoped to ownership/complexity; new task 3.4 and decision [9] give the duplicated impl bodies a `macro_rules!` owner with the existing handshake-metadata and debug-level tests as guards; Verification now shows the duplication arithmetic (4.0% → ≈ 2.9% via `wire.rs` → ≈ 0 from this scope via 3.4).
- **Promotes to ADR:** no
