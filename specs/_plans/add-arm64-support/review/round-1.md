# Plan Review Findings: add-arm64-support (round 1)

## Summary
- Axes checked: 6/6
- Total findings: 9 (Blockers: 1, Advisory: 8)
- Intent Fidelity blockers: 0

## Premortem

Three ways this plan fails six months out:

1. **P0 never lands because its execution model is undefined.** PR #51 lives on an outside contributor's cross-repo fork (`realtdegen:feat/aarch64-arm64-support`), unfetchable from `origin`. `main` does NOT contain the arch-neutral Dockerfile. The plan assumes "PR #51's four-file change is already present on the branch," but the speq-implement flow works on an in-repo branch off `main` where those four files are unchanged — and there is no task to apply the diff. Everything depends on P0. → Feasibility.
2. **The `SCRIPT_LANGUAGES` `#`-fragment invariant drifts.** The executable-path-no-leading-slash rule (a `22002`-crash-if-wrong decision) is hardcoded inline in `scripts/install.sh:123` and will be re-implemented independently in `scripts/install-personal.sh`. A future edit to one silently crashes the other. → Design Depth (information leakage).
3. **A false technical fact is enshrined in an ADR.** Decision [4] (promotes to ADR) justifies removing `targets/*.json` partly on a "stale data-layout"; that data-layout is byte-for-byte identical to rustc 1.94.1's output. → Requirement Quality.

## Round-1 Blocker Recheck
Not applicable (round 1).

## Intent Fidelity

[no objection — axis checked: all eight issue-#78 sections §0–§7 map to phases P0–P7 with none dropped; the four-triple `about.toml` change is explicitly sanctioned by issue §2 ("verify the triple choice … the musl-only pin may already under-report gnu-gated deps"), confirmed by Agent B that the shipped binary is glibc while `about.toml:24` pins musl-only — so it is a fix, not scope creep; the §0 slim-image spec-delta decision was explicitly deferred to planner judgment by the user's interview answer and is exercised in decision [1]; task 1.5's extra edit of `docs/cargo-ecosystem.md` is the same de-hardcoding concern, not creep.]

## Feasibility

#### [HIDDEN_DEPENDENCY] BLOCKER
- Location: plan.md § "P0 — Land the arch-agnostic SLC build"; decision-log.md [10]
- Issue: P0 is the foundation ("P0 → everything") but its execution model is undefined. PR #51 is a cross-repository fork PR (`isCrossRepository: true`, owner `realtdegen`), the branch is unfetchable from `origin` (`git` confirms "unknown revision"), and `main` still carries the x86_64-hardcoded Dockerfile (the recorded `slim-image` Background names `ld-linux-x86-64.so.2`; Agent C confirmed only the x86_64 `targets` JSON exists on main). Task 0.0's premise — "PR #51's four-file change … is already present on the branch" — holds only on realtdegen's fork. The two viable execution readings both break as written: (A) work on realtdegen's fork branch → the implementer cannot commit to it and task 0.4 "narrow the PR #51 description" needs write access to another user's PR; (B) work on a fresh in-repo branch off `main` → the four-file arch-neutral change is NOT present and no task applies it, and task 0.4 is orphaned because a superseding in-repo PR is not PR #51. Either way, P0 cannot produce the arch-neutral build as written.
- Fix: In plan.md § P0, state which branch model applies. If a new in-repo branch: add an explicit first task "apply PR #51's four-file diff (`.cargo/config.toml`, `Dockerfile.alpine`, `rust-toolchain.toml`, `targets/aarch64-unknown-linux-musl-dylib.json`) onto the working branch," and drop or rewrite task 0.4 (it does not apply to an in-repo PR). If working on realtdegen's fork: add a task establishing write access (maintainer re-push or allow-edits) and keep 0.4. Update decision-log.md [10] to record the chosen model.

#### [HIDDEN_DEPENDENCY] ADVISORY
- Location: plan.md § "P4 — CI" task 4.3
- Issue: Task 4.3 says "Coordinate the leg placement with the in-flight CI restructuring (#67–#70)" but sets no ordering. Agent D confirmed #67–#70 are OPEN and that #67 ("unblock build-slc") and #69 ("flatten the job graph") restructure the very `build-slc`/unit-test jobs P3 (matrix) and P4 (arm leg) attach to. If #67–#70 land during or after this plan, P3/P4 need rework against a moved job graph.
- Fix: In plan.md § P4 (or § Dependencies), state the intended ordering — either this plan lands before #67–#70 (and they rebase onto the arm matrix), or this plan waits for #67–#70 and P3/P4 attach to the flattened graph. Name the concrete dependency, not "coordinate."

#### [UNSTATED_ASSUMPTION] ADVISORY
- Location: plan.md § "Cross-cutting release hygiene"; § Parallelization (Group A)
- Issue: Group A schedules P1, P2, P4, P5, P7 in parallel, and every phase bumps `[workspace.package].version`, the pinned `exasol-udf-sdk` dep, and `Cargo.lock`. Developed in parallel these collide on the same three artifacts; two phases can also target the same next version.
- Fix: In plan.md § "Cross-cutting release hygiene", state that version bumps serialize on landing — each phase re-bumps from the then-current version at merge time — so parallel development is fine but the bump is resolved last, at rebase.

## Requirement Quality

#### [COMPLETENESS_GAP] ADVISORY
- Location: decision-log.md [4]; plan.md § "Dead Code Removal" (targets JSON row); § Consequences (targets/*.json row)
- Issue: The removal is justified partly on "The x86_64 data-layout is stale for 1.94." Agent C verified with `rustc 1.94.1 --print target-spec-json` that `targets/x86_64-unknown-linux-musl-dylib.json`'s `data-layout` is byte-for-byte identical to what rustc 1.94.1 emits; the only differing field (`crt-static-default`) is a deliberate dylib customization, not staleness. Decision [4] promotes to ADR, so the false claim would be recorded permanently. (The removal itself remains sound on the "no rustc consumer" ground: Agent C confirmed `COPY targets/ ./targets/` in `Dockerfile.alpine:18` is the only reference; the `-Zbuild-std` comment at `crates/exasol-udf-sdk/build.rs:8` does not name the JSON.)
- Fix: Delete the "stale data-layout" justification from decision-log.md [4], plan.md § "Dead Code Removal", and § Consequences. Rest the §7 removal solely on "no in-repo rustc consumer; the sole reference is the `Dockerfile.alpine` `COPY targets/` line." Optionally correct [4] to note the `exasol-udf-sdk/build.rs:8` `-Zbuild-std` comment does not reference the JSON and so needs no cleanup.

#### [COMPLETENESS_GAP] ADVISORY
- Location: specs/_plans/add-arm64-support/container/slim-image/spec.md § "SLC builds natively for the host architecture"; plan.md § "Scenario Coverage"
- Issue: The scenario's normative clause "an empty derived multiarch triplet or loader path MUST fail the build with a named error" (implemented by task 0.3's fail-fast guard) has no row in the Scenario Coverage table and no mapped test. A normative MUST with no pass/fail test is unverified.
- Fix: Add a Scenario Coverage row for the fail-fast guard — either an automated docker-build test that stubs an empty `TRIPLET`/`LOADER` and asserts the named error, or an explicit "code-review / manual" verification entry — and reference task 0.3.

#### [COMPLETENESS_GAP] ADVISORY
- Location: plan.md § "Scenario Coverage" (cargo-exaudf integration rows); tasks 1.1, 1.4
- Issue: Agent A confirmed the existing integration test `build_produces_musl_so` is `#[ignore]`d (`crates/cargo-exasol-udf/tests/build.rs:35`, "requires musl toolchain and cargo; run with --ignored"); the planned `build_installs_missing_target` and `build_honors_target_override` sit in the same file and will be ignored likewise. Plain `cargo test` (the § Checklist step) does not run ignored tests, and no task wires an `--ignored` run into CI. The coverage table lists these as covering tests, overstating automated regression protection; the only automated §1 coverage is the pure unit test `host_triple_maps_arch` in `build_tests.rs`.
- Fix: In plan.md, either add a task/CI leg that runs `cargo test -p cargo-exasol-udf -- --ignored` on a musl-capable runner, or annotate the three integration rows in § "Scenario Coverage" as "ignored — local/manual only," so the automated coverage claim matches reality.

#### [COMPLETENESS_GAP] ADVISORY
- Location: plan.md § "Scenario Coverage" (personal-install rows for "Connection details are read fresh on every run" and "Registration is system-scoped and preserves existing entries")
- Issue: Both rows are marked Test Type = Unit, but each scenario carries a live-only sub-clause the named sourced-function test cannot exercise: "stays correct across an `exasol stop`/`start` cycle that reassigns the port" and "re-running the install MUST be idempotent across an `exasol stop`/`start` cycle." Restart behavior requires a live Personal deployment. The unit test covers only single-call reading and string preservation.
- Fix: In plan.md § "Scenario Coverage", change these two rows to Test Type = Unit + Manual, and add the restart-cycle re-run to the § "Manual Testing" table so the idempotency-across-restart clause has an explicit manual verification.

## Task Breakdown

[no objection on traceability — axis checked: every spec-delta scenario maps to an implementing task (cargo-exaudf→P1.1–1.5; slim-image→P0.3 plus PR #51's landed builder; crate-license-notices→P2.1–2.2; personal-install→P5.1–5.2) and every task implements in-scope work.]

#### [TASK_GRANULARITY] ADVISORY
- Location: plan.md § "P5 — Exasol Personal install path" task 5.1
- Issue: Task 5.1 (one `[expert]` task) bundles four independently-failing concerns: (a) build-or-accept the aarch64 tarball, (b) read `sshPort`/key fresh from `deployment.json`, (c) `scp` + extract into `/var/lib/exa/bucketfs/.../` (filesystem BucketFS reconciliation), and (d) assemble + issue `ALTER SYSTEM SET SCRIPT_LANGUAGES` preserving existing entries with the correct `#` fragment. Only (b) and (d) get unit tests (5.2); (a) and (c) are manual and unverifiable as a single unit.
- Fix: Split task 5.1 into at least two tasks in plan.md § P5 — one for transport/extraction (tarball acquisition + scp + filesystem reconciliation) and one for connection-detail reading + registration-string assembly + `ALTER SYSTEM` — so each is separately verifiable against its scenario.

## Design Depth

[no objection on the core design — axis checked: parameterizing one pipeline by host-derived arch (`std::env::consts::ARCH`, `gcc -print-multiarch`, `readelf -l` PT_INTERP) rather than forking per-arch paths is the deep, drift-resistant choice and aligns with auto-detection over a config flag; the §0 slim-image spec delta is consistent with the existing spec's already-mechanics-heavy granularity (the recorded "Builder toolchain and glibc runtime" scenario already names `cp -L`, `/glibc-rt/`, `zeromq-src`) and the spec-vs-mechanics call was user-deferred, so it is not re-litigated here.]

#### [INFORMATION_LEAKAGE] ADVISORY
- Location: plan.md § "P5" tasks 5.1–5.2; decision-log.md [7]
- Issue: The `SCRIPT_LANGUAGES` `#`-fragment format — points at the `exaudfclient` executable, no leading slash, a `22002`-crash-if-wrong invariant preserved by decision [8] — is already hardcoded inline at `scripts/install.sh:123` (`…#buckets/${BFS_SERVICE}/${BUCKET}/slc/${SLC_NAME}/exaudf/exaudfclient`). Task 5.2 factors the fragment assembly into "pure shell functions" only inside `install-personal.sh`, leaving the same invariant expressed in two scripts with nothing enforcing agreement. Decision [7]'s "separate script" choice is fine, but it need not duplicate the fragment rule.
- Fix: In plan.md § P5, add a task to extract the `#`-fragment assembly (executable path, no leading slash) into a shared sourced helper (e.g. `scripts/lib/script_languages.sh`) that both `install.sh` and `install-personal.sh` source, giving the invariant a single owner, and point the 5.2 unit tests at that helper.

## Prose Quality

[no objection — axis checked: plan.md leads with a BLUF Summary within the two-sentence cap, uses statement-style phase headings, and keeps rationale terse; spec-delta feature-description lines are single-sentence and active-voice; no filler, hedging, or ambiguous pronouns that block action were found.]
