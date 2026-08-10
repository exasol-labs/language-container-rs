# Plan Review Findings: fix-glibc-cdylib-build-model (round 2)

## Summary
- Axes checked: 6/6
- Total findings: 1 (Blockers: 0, Advisory: 1)
- Intent Fidelity blockers: 0

## Round-1 Blocker Recheck

- **Resolved: [HIDDEN_DEPENDENCY] F1 — the un-ignored `build_produces_host_cdylib` gate could not build the scaffolded crate.** The fix is sound and verified end-to-end:
  - Task 1.3 repoints `new.rs:43-44` pins `"0.1"` → `"0.21"` (both `exasol-udf-sdk` and `exasol-udf-macros`); `"0.21"` (^0.21) is satisfied by the workspace SDK `0.21.3` (`Cargo.toml` `[workspace.package].version = "0.21.3"`).
  - Task 1.1 injects `[patch.crates-io]` pointing both crates at the in-repo paths via `env!("CARGO_MANIFEST_DIR")` + `/../..`. Verified the target crate dirs exist (`crates/exasol-udf-sdk`, `crates/exasol-udf-macros`) and their package names (`exasol-udf-sdk`, `exasol-udf-macros`) exactly match the scaffold's dependency names, so the patch redirects to the local crates. The path is absolute (from `CARGO_MANIFEST_DIR`), so it resolves regardless of the tempdir location.
  - The scaffold compiles against 0.21.3: `crates/exasol-udf-sdk/src/lib.rs` re-exports `context::UdfContext` and `error::UdfError` (the exact module paths the scaffold `use`s), and the macro supports the scaffold's `fn run(...) -> Result<Option<i64>, UdfError>` shape (proven by `crates/exasol-udf-macros/tests/output_shape.rs:23,28` and `test-udfs/set-sum/src/lib.rs:11`). Edition 2024 is supported by the pinned channel 1.94.
  - The patch mechanism actually *gates* the pin: cargo requires the local version (0.21.3) to satisfy the scaffold requirement, so a stale pin like `"0.1"` would fail patch resolution and redden the gate rather than paper over it. The `cargo-exaudf` scenario "new scaffolds a buildable UDF crate" (DELTA:CHANGED) records the contract; the scenario↔test mapping (plan.md:106) is consistent. `tests/new.rs` asserts only `cdylib` / crate name / presence of `exasol-udf-sdk`, not the version literal, so task 1.3 does not break it.

- **Resolved: 4 round-1 advisories, no new blocker introduced.**
  - about.toml deferral: plan.md § Dead Code Removal NOTE + decision-log [5] + a `[plan-review]` entry record the deferral to PR #79's P2. Confirmed the deferral does not break #80 CI: `cargo about` resolves `cfg(target_*)` from the built-in target list, needing no installed rustup component, so removing the musl toolchain target (task 2.2) leaves `dist/generate-licenses.sh` runnable.
  - Override test un-ignored: task 1.1 reads the host triple from `rustc -vV` and runs `--target <host-triple>`. Passing `--target` (even the host triple) makes cargo emit under `target/<triple>/release/`, and the host target needs no `rustup target add`, so the test runs unconditionally in CI. Feasible.
  - "glibc-dynamic" qualifier dropped: `tools/cargo-exaudf/spec.md:50` now reads "a cdylib that exports at least one `__exa_udf_entry_<NAME>` symbol". The remaining "glibc-dynamic" mentions are descriptive Feature/Background prose, not scenario THEN assertions, so they need no test.
  - Task 3.4 deleted + renumber: `docs/installation.md` has no musl/support-matrix framing on `main` (confirmed); the renumbered task 3.4 (architecture.md) targets real content — `specs/architecture.md:41` "static musl cdylib" and `:91` "build musl .so" both exist. No `3.5` survives as a live task reference (the only hit is the decision-log entry describing the renumber). ADR 024 is free (`_decision/` max is 023).

## Intent Fidelity
[no objection — axis checked: the plan still operationalizes the interview decisions — glibc-default `cargo exasol-udf build` + `--target <triple>` override (decision-log [1]), removal of the user-enumerated musl config (decision [2]), #80-first / #79-rebase coordination (decision [3],[5]), and the un-ignored regression gate (decision [3]). `specs/mission.md` on disk reads the glibc-dynamic-cdylib model throughout (L11-12, L29, L51, L95), confirming the out-of-delta reconciliation. No scope silently dropped or added since round 1.]

## Feasibility

#### [UNSTATED_ASSUMPTION] ADVISORY
- Location: tasks.md § Group A task 1.3 (`new.rs` pin hardcoded `"0.21"`); interacts with CLAUDE.md's mandatory version bump
- Issue: Task 1.3 hardcodes the scaffold pin `"0.21"` to track the current `0.21.3`. The project rule "Bump `[workspace.package].version` on every change (SemVer)" means this plan will bump the version, and decision [1] "Promotes to ADR: yes" flags a behavioral change to the public CLI default — under pre-1.0 SemVer a breaking change bumps the minor (`0.21.x` → `0.22.0`). If the bump crosses to `0.22.0` and the scaffold pin stays `"0.21"`, the `[patch.crates-io]` requirement `"0.21"` (`< 0.22.0`) is no longer satisfied by the local `0.22.0` crate, so `build_produces_host_cdylib` fails patch resolution — the same defect class as round-1 F1, re-triggered by the plan's own version bump. This is not a silent-ship risk: the un-ignored gate catches it in CI and blocks merge. Severity is advisory because the version-bump decision lives outside the plan (CLAUDE.md / `/speq:implement-pr`), the fix is a one-line pin edit, and the gate enforces the coupling. Task 1.3's stated principle ("the current SDK major.minor line") is self-correcting if the implementer reads it as authoritative over the literal.
- Fix: In tasks.md task 1.3 (and optionally plan.md § Dead Code Removal or § Dependencies), add a one-line note: the scaffold pin in `new.rs` MUST track the major.minor of whatever `[workspace.package].version` this plan's SemVer bump lands on — if the bump crosses to `0.22.x`, set the pin to `"0.22"`; the un-ignored `build_produces_host_cdylib` gate enforces this coupling in CI.

## Requirement Quality
[no objection — axis checked: `speq feature validate tools/cargo-exaudf` returns 0 errors / 0 warnings. The two round-1 advisories (override-test executability, unverified "glibc-dynamic" qualifier) are resolved. The DELTA:CHANGED "new scaffolds a buildable UDF crate" requirement "pins track the current SDK major.minor line" is verifiable: cargo patch resolution rejects a pin incompatible with the local SDK version, so `build_produces_host_cdylib` reddens on a stale pin. Example-fixture DELTA:CHANGED scenarios are wording-only reconciliations of already-shipped glibc-cdylib behavior, verified by the existing `it` `db_roundtrip_all_scenarios` suite.]

## Task Breakdown
[no objection — axis checked: every docs/reference task has a real target on `main` — README L61/L68/L119, `docs/writing-a-udf.md` L9-11/L598/L602/L614, `docs/cargo-ecosystem.md` L84/L87, `specs/architecture.md` L41/L91 all still carry musl wording. Task 1.2's `MUSL_TARGET`/`ensure_musl_target` deletion is safe: both symbols are confined to `build.rs` (no external reference). No CI step invokes `cargo build --target musl` (the sole `--target` in ci.yml is a Docker build stage), so Group B config removals break nothing. Group A's within-group sequence (1.1 test-first → 1.2 build.rs → 1.3 new.rs) reaches a consistent green state; Groups A/B/C touch disjoint files. Renumber left no orphaned task reference.]

## Design Depth
[no objection — axis checked: unchanged from round 1 — a required flag reduced to a sensible default (host glibc) with a `--target` escape hatch; `build.rs` remains the single owner of target selection; no new module, boundary, or business-logic-on-mechanism dependency introduced.]

## Prose Quality
[no objection — axis checked: plan.md Summary is two BLUF sentences; the new Dead Code Removal NOTE and decision-log `[plan-review]` entries state the deferral/coordination tersely without hedging or filler. Governed prose (Summary/Goals/Non-Goals/Impact, decision rationales) carries no ambiguity that renders a requirement non-actionable; verification-table cells are tables (ungoverned).]
