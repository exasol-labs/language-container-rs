# Plan Review Findings: fix-glibc-cdylib-build-model (round 1)

## Summary
- Axes checked: 6/6
- Total findings: 5 (Blockers: 1, Advisory: 4)
- Intent Fidelity blockers: 0

## Premortem

1. **The root-cause gate never actually ran.** Six months on, `cargo exasol-udf build` is silently broken again by a refactor. CI stayed green because `build_produces_host_cdylib` — the test the user demanded be un-ignored as a regression gate — could not build the scaffolded crate (its `exasol-udf-sdk = "0.1"` pin does not resolve against the 0.21.x workspace SDK), so it was quietly re-`#[ignore]`d or weakened to assert nothing real. The exact failure #80 set out to prevent recurs. → Feasibility BLOCKER F1.
2. **The shipped license bundle under-reports a glibc dependency.** `THIRD-PARTY-LICENSES.md` omits a `x86_64-unknown-linux-gnu`-gated crate's notice because `about.toml` still computes the crate graph for the abandoned musl target — a compliance gap in the distributed tarball, and a musl artifact the "one consistent model" reconciliation left behind. → Requirement Quality R1.
3. **An implementer invents a support matrix that collides with PR #79.** Told task 3.4 reconciles installation.md's "musl-primary support matrix," an implementer writes a matrix section that does not exist on `main`, colliding with the one PR #79 adds — duplicated, contradictory content, the opposite of "one consistent statement." → Task Breakdown T1.

## Intent Fidelity

[no objection — axis checked: the plan adopts the interview-decided glibc-default + `--target` override (decision-log [1]), removes the user-enumerated musl config (rust-toolchain.toml target, `.cargo/config.toml` stanza, `targets/*.json`, Dockerfile COPY), and un-ignores a build-invoking gate — Findings A+D. Value prop preserved: `specs/mission.md` on disk now reads "glibc-dynamic `cdylib` `.so` ... no in-container toolchain and no runtime compilation" (L11-12, L51), confirming the mission was reconciled out-of-delta as stated. Deferring the `cargo exaudf` vs `cargo exasol-udf` naming (decision [6]) is scoped-out with rationale, not silent drop. The `Supersedes` target "cargo-exaudf hides the musl target triple from authors" exists at `specs/_decision/002-add-v2-rust-udf-complete.md:79`, and ADR 024 is free (`_decision/` max is 023) — coordination decision [5] is internally consistent.]

## Feasibility

#### [HIDDEN_DEPENDENCY] BLOCKER
- Location: tasks.md § Group A task 1.1 + plan.md § Verification Checklist ("CLI build test → `build_produces_host_cdylib` passes (no longer ignored)"); root cause in `crates/cargo-exasol-udf/src/new.rs:43-44`
- Issue: The plan's centerpiece — un-ignoring `build_produces_host_cdylib` so it runs in CI — cannot pass as written. The scaffold that `new` writes pins `exasol-udf-sdk = { version = "0.1", features = [] }` and `exasol-udf-macros = { version = "0.1" }` (`new.rs:43-44`), but the workspace/current SDK is `0.21.3` (`Cargo.toml:75,83`) and the `__exa_udf_entry_<NAME>` named-entry ABI the build's post-verification requires (`build.rs:49-59`, via `enumerate_entry_symbols`) postdates 0.1 (added in ADR 016). The un-ignored test scaffolds a crate and runs a real `cargo build --release` on it (test `build_produces_musl_so` → renamed `build_produces_host_cdylib`, `tests/build.rs:34-61`). A hermetic CI build of that scaffold either (a) fails to resolve `exasol-udf-sdk = "0.1"` (no matching version, or unpublished pre-release), or (b) fails to compile the current-API `src/lib.rs` (`use exasol_udf_sdk::context::UdfContext`, edition 2024, `Result<Option<i64>, UdfError>`) against an ancient 0.1 SDK, or (c) builds a `.so` lacking `__exa_udf_entry_<NAME>` so the CLI's verification returns `Err`. The plan misattributes the `#[ignore]` solely to musl (decision-log [3]: "the glibc default build needs no special toolchain, so it can run unignored in CI") and adds no task to make a scaffolded crate build against the current SDK. The user's explicit "root-cause gate" ask is therefore undeliverable as specified, and the plan is untestable as written.
- Fix: Add a Group A task (before or with 1.2) that makes a freshly-scaffolded crate build against the current SDK: (1) repoint `new.rs`'s `exasol-udf-sdk`/`exasol-udf-macros` version pins to the current published/workspace major.minor line (e.g. `"0.21"`), and (2) make `build_produces_host_cdylib` build the scaffolded crate against the local workspace SDK regardless of crates.io state — inject a `[patch.crates-io]` (or path) entry for `exasol-udf-sdk`/`exasol-udf-macros` into the test's scaffolded `Cargo.toml` pointing at the in-repo crates — so the un-ignored gate compiles green on a standard runner with no dependency on a published version. Add a `cargo-exaudf` spec scenario (or amend "new scaffolds a buildable UDF crate") capturing the scaffold's SDK version contract. If neither is feasible in-scope, the plan must not claim the gate runs un-ignored in CI.

## Requirement Quality

#### [COMPLETENESS_GAP] ADVISORY
- Location: `about.toml:24` (`targets = ["x86_64-unknown-linux-musl"]`); absent from plan.md § Dead Code Removal and all tasks
- Issue: `about.toml` still pins the license-bundle crate graph to `x86_64-unknown-linux-musl` — an active musl config, consumed by `dist/generate-licenses.sh` (CI step "Generate license bundles", `.github/workflows/ci.yml:277-278`) to produce `THIRD-PARTY-LICENSES.md`, a mission core capability (value-prop item 5). The plan removes the toolchain's musl target but leaves the license bundle computed for that abandoned target, so the shipped attribution can under-report `x86_64-unknown-linux-gnu`-gated dependencies — the exact concern the recorded arm64 plan flagged (`specs/_recorded/003-add-arm64-support/`). This does not break `#80`'s CI (cargo-about resolves target cfg without the rustup component), so it is advisory, but it leaves the "one consistent build model" internally inconsistent. It is unmentioned — neither reconciled nor explicitly deferred.
- Fix: Add a Group B task to repoint `about.toml`'s `targets` to `x86_64-unknown-linux-gnu` (the glibc model), OR add a NOTE (mirroring task 2.4's `targets/*.json` overlap note) explicitly deferring `about.toml` to PR #79's task 2.1 with rationale. Record the choice in plan.md § Dead Code Removal or § Dependencies.

#### [AMBIGUOUS_REQUIREMENT] ADVISORY
- Location: `tools/cargo-exaudf/spec.md` § Scenario "build honors an explicit target override" (DELTA:NEW) + plan.md § Verification "build_honors_target_override (`#[ignore]` ...)"
- Issue: This NEW recorded scenario (a `MUST`) is mapped only to `build_honors_target_override`, which stays `#[ignore]`d and uses a placeholder `<triple>` throughout — so a recorded requirement has no executing CI test and no concrete pass/fail value. It is not verifiable as written.
- Fix: In tasks.md 1.1 and plan.md § Verification, make the override test run in CI by using the host's own target triple as the concrete `--target` argument (installed by definition), asserting the `target/<host-triple>/release/lib<crate>.so` path, and remove its `#[ignore]`. Otherwise state in the scenario and the verification table that it is manually verified, and why an executing test is impossible.

#### [COMPLETENESS_GAP] ADVISORY
- Location: `tools/cargo-exaudf/spec.md` § Scenario "build produces a loadable host cdylib" (DELTA:NEW) vs tasks.md 1.1
- Issue: The scenario asserts the artifact "MUST be a glibc-dynamic cdylib", but task 1.1's mapped assertions cover only the printed path and the exported `__exa_udf_entry_<NAME>` symbol — the "glibc-dynamic" property (dynamic interpreter present, not a static binary) is never asserted. On a glibc CI host it is implicitly true, so this is minor, but the recorded `MUST` is unverified.
- Fix: Either drop the "glibc-dynamic" qualifier from the scenario's THEN clause (leaving "a cdylib that exports at least one `__exa_udf_entry_<NAME>` symbol"), or add an assertion in task 1.1 that the produced `.so` is dynamically linked (e.g. has a `PT_INTERP` / dynamic segment).

## Task Breakdown

#### [TRACEABILITY_GAP] ADVISORY
- Location: tasks.md § Group C task 3.4; decision-log [4]
- Issue: Task 3.4 reconciles "the support-matrix wording that frames the artifact as musl-primary" in `docs/installation.md`, but that file on current `main` (174 lines) contains no `musl`, `static`, `cdylib`, `glibc`, or support-matrix framing — the only build mention (L174, "building the `.so` with `cargo-exasol-udf`") is already build-model-neutral. The plan is authored against current `main` (decision A), so the task has no traceable target; the "support matrix" and "arch/Personal-install notes" it references are additions PR #79 makes. As written it is a no-op, or risks an implementer inventing a section that collides with #79 (premortem 3). All genuine `docs/`/`README.md` musl references are already covered by tasks 3.1-3.3 (verified), so no reconciliation coverage is lost by removing 3.4.
- Fix: Delete task 3.4 and the `docs/installation.md` mention in decision-log [4]; if a specific line needs the glibc qualifier, name it concretely instead. Note in the parallelization/scope text that installation.md's build-model wording is unaffected on `main`.

[no objection on WBS structure — axis checked: parallel groups A (code: `build.rs`, `tests/build.rs`), B (config: `Cargo.toml`, `rust-toolchain.toml`, `.cargo/config.toml`, `targets/`, `Dockerfile.alpine`), and C (docs/reference) touch disjoint files and are safely concurrent; the two `[expert]` tags (1.1, 1.2) correctly cover the FFI/ABI-touching build logic; task 2.4's Dockerfile claim ("already `rm`s rust-toolchain.toml and builds glibc with no `--target`") is confirmed accurate at `Dockerfile.alpine:22-24`, and the `targets/*.json` overlap with PR #79 P7 is explicitly coordinated.]

## Design Depth

[no objection — axis checked: the change reduces a required decision to a sensible default with an escape hatch (glibc default, `--target` override), matching design-philosophy "a config parameter is a decision the module declined to make"; `build.rs` remains the single owner of target selection (no leakage introduced); no new module, boundary, or business-logic-on-mechanism dependency is created.]

## Prose Quality

[no objection — axis checked: plan.md Summary is two BLUF sentences; Goals/Non-Goals/Impact are terse and verb-led; decision-log rationales state the tradeoff without hedging or filler. No PROSE_BLOAT or PROSE_UNCLEAR that renders a requirement non-actionable.]
