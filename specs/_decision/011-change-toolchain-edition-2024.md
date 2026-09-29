# Decisions: change-toolchain-edition-2024

## ADR: Raise the workspace MSRV pin from 1.84 to 1.92

**ID:** raise-workspace-msrv-1-84-to-1-92
**Plan:** `change-toolchain-edition-2024`
**Status:** Accepted

### Context

The workspace `rust-toolchain.toml` was pinned at `channel = "1.84"`. This was a false floor: `connect-back` feature users already required Rust >= 1.85 (arrow 58 transitively pulls in edition-2024 crates that Rust 1.84 cannot parse), so those crates were excluded from `default-members` and built explicitly with `cargo +1.91` overrides. The CI and Docker builder images always ran 1.91. Three toolchain versions were in play simultaneously — workspace pin (1.84), CI/builder (1.91), and the effective floor for all members (1.85+). Raising the pin to 1.92 unifies all three on one version. 1.92 is the minimum that clears the edition-2024 floor (edition 2024 stabilized in 1.85) and matches the `rust-version = "1.92"` floor on the strata-rs iceberg side. The whole stack is unreleased, so raising the floor carries no downstream cost.

### Decision

Bump `rust-toolchain.toml` `channel` from `"1.84"` to `"1.92"`. Collapse the `default-members` split: move `connect-back-query`, `connect-back-insert`, `connect-back-crunch`, `connect-back-cluster-ip`, and `spike-connect` into `default-members`. Drop all `cargo +1.91` / `cargo +1.92` override prefixes in CI and scripts. Remove the per-crate `rust-version = "1.85"` from `crates/it` — the toolchain pin becomes the single MSRV source of truth.

### Options Considered

| Option | Verdict |
|--------|---------|
| Raise pin to 1.92; unify all three toolchains | ✓ Chosen — eliminates the `default-members` split and all `cargo +N` overrides; 1.92 satisfies the edition-2024 floor and the strata-rs iceberg floor; no downstream cost for an unreleased stack |
| Keep pin at 1.84; retain `default-members` split and `cargo +N` overrides | ✗ Rejected — the 1.84 floor was already effectively broken; maintaining the three-toolchain split is dead weight |
| Delete the pin entirely (float to latest stable) | ✗ Rejected — a pin is still wanted so `rustup` selects 1.92 deterministically; the Docker `rm rust-toolchain.toml` fallback to the image toolchain remains a clean no-op |

### Consequences

All workspace members including the `connect-back` crates build under plain `cargo` with no `+N` overrides. The `default-members` exclusions for `connect-back-*` and `spike-connect` are removed. The only remaining exclusion from `default-members` is `crates/it`, which stays out because it requires a live Exasol Docker container — a runtime dependency, not a toolchain constraint.

## ADR: Migrate the whole workspace to edition 2024

**ID:** migrate-workspace-edition-2024
**Plan:** `change-toolchain-edition-2024`
**Status:** Accepted

### Context

All 19 workspace crates (`crates/*/` and `test-udfs/*/`) were at `edition = "2021"`. Once the MSRV pin is raised to 1.92 (ADR-033), edition 2024 is available across the whole workspace. The north-star goal is one toolchain and one edition so a developer runs plain `cargo build` / `cargo test` with zero `+N` flags and zero edition juggling. Mixed editions (some crates 2021, some 2024) leave exactly the complexity the toolchain unification is meant to eliminate.

### Decision

Bump `edition` from `"2021"` to `"2024"` in all 19 crate manifests. Run `cargo fix --edition` per crate under the 1.92 toolchain to apply mechanical migrations. The single non-trivial code change edition 2024 requires in this workspace is the proc-macro FFI attribute, captured in ADR-035.

### Options Considered

| Option | Verdict |
|--------|---------|
| Migrate all 19 crates to edition 2024 | ✓ Chosen — one toolchain + one edition eliminates dual-edition complexity; the whole stack is unreleased so there is no backward-compatibility cost; `cargo fix --edition` handles mechanical migrations |
| Keep mixed editions (2021 for core crates, 2024 for test-udfs only) | ✗ Rejected — defeats the one-edition DX goal; forces per-invocation edition awareness to persist |
| Pin at latest stable instead of 1.92 | ✗ Rejected — 1.92 is the justified minimum satisfying both the edition-2024 floor and the strata-rs iceberg floor; pinning higher buys nothing and drifts from the strata-rs side |

### Consequences

All 19 crate manifests declare `edition = "2024"`. The `rust-version = "1.85"` field on `crates/it` is removed; the toolchain pin is the single MSRV declaration. The success criterion is satisfied: a developer runs `cargo build` / `cargo test` with zero `+N` flags and zero edition juggling.

## ADR: Proc-macro emits #[unsafe(no_mangle)] for the UDF entry point

**ID:** proc-macro-unsafe-no-mangle-entry-point
**Plan:** `change-toolchain-edition-2024`
**Status:** Accepted

### Context

The `exasol-udf-macros` proc-macro emitted a bare `#[no_mangle]` for the generated `__exa_udf_entry` FFI entry point. Edition 2024 promotes `#[no_mangle]`, `#[export_name]`, and `#[link_section]` to unsafe attributes. Proc-macro-emitted tokens are interpreted in the call-site crate's edition: once any consuming crate (test-udf crates) becomes edition 2024, the bare `#[no_mangle]` errors at the call site, not in the macro crate. Gating the attribute on the call-site edition from inside a proc-macro is impossible — proc-macros cannot reliably read the consumer's edition.

### Decision

Change the `exasol-udf-macros` proc-macro to emit `#[unsafe(no_mangle)]` instead of bare `#[no_mangle]` for the generated `__exa_udf_entry`. The change is unconditional: `#[unsafe(no_mangle)]` is valid on Rust >= 1.82 in both edition 2021 and edition 2024. No other bare `#[no_mangle]` / `#[export_name]` / `#[link_section]` is emitted; the existing `unsafe extern "C"` shim blocks are already edition-2024-correct and are unchanged.

### Options Considered

| Option | Verdict |
|--------|---------|
| Emit `#[unsafe(no_mangle)]` unconditionally | ✓ Chosen — valid on >= 1.82 in both editions; unconditional is simpler and future-proof; proved correct by the `dispatch.rs` test loading a macro-generated edition-2024 `.so` and running the full cycle |
| Gate the attribute on call-site edition inside the macro | ✗ Rejected — proc-macros cannot reliably know the consumer's edition; fragile and unnecessary |
| Keep `#[no_mangle]`; hold test-udf crates at edition 2021 | ✗ Rejected — defeats the one-edition goal; call-site error would persist for any future edition-2024 consumer |

### Consequences

The macro emits `#[unsafe(no_mangle)]` for every generated UDF entry point. Edition-2024 call-site crates compile without error. The `dispatch.rs` integration test loads a real macro-generated edition-2024 `.so` and drives it through the `__exa_udf_entry` loader, proving the entry point still exports and dispatches correctly.

## ADR: Spec-delta-free plans are legitimate; specs must not encode implementation detail

**ID:** spec-delta-free-plans-legitimate
**Plan:** `change-toolchain-edition-2024`
**Status:** Accepted

### Context

The initial drafts of this plan authored spec deltas asserting specific version strings (`rust:1.91-bookworm` → `rust:1.92-bookworm`, `channel = "1.84"` → `"1.92"`), edition literal values, and the macro attribute spelling (`#[no_mangle]` vs `#[unsafe(no_mangle)]`) directly in scenario specs. These are implementation details — not developer-observable behavior. Encoding them in specs creates maintenance debt: every toolchain bump would require a spec delta and a re-record cycle, and the spec library would become a mirror of Cargo.toml and Dockerfile rather than a description of system behavior.

### Decision

Remove all three spec delta files from this plan, making it a spec-delta-free plan (`plan.md` + `decision-log.md` only). Specs describe developer-observable behavior; version numbers, toolchain channels, Docker image tags, attribute spellings, edition values, and Cargo manifest fields are implementation details and must not appear in scenario specs. A plan that carries only a decision log and implementation tasks is legitimate when the work has no observable behavioral delta. As a recording-phase cleanup, version literals that had leaked into prior specs are stripped from the permanent spec library on record.

### Options Considered

| Option | Verdict |
|--------|---------|
| Spec-delta-free plan; strip leaked version literals from existing specs on record | ✓ Chosen — keeps the spec library focused on behavior; eliminates maintenance debt from version-literal churn |
| Keep the spec deltas with version-string assertions | ✗ Rejected — version strings are implementation detail; every future toolchain bump would force a spec record cycle |
| Rewrite the deltas to describe behavior abstractly | ✗ Rejected — at this granularity the change is pure implementation detail (toolchain version, edition value, FFI attribute spelling) with no observable behavioral surface to assert |

### Consequences

The spec library's scenario assertions describe what the system does, not how it is built. Implementation details (toolchain version, Rust edition, Docker image tag, FFI attribute form) live only in code and `plan.md` / `decision-log.md` history. Spec-delta-free plans are an established pattern for infrastructure-level changes that preserve all shipped behavior. Prior version-literal leakage (the "Rust toolchain is pinned" scenario with `channel = "1.84"`, the `[workspace.dependencies]` scenario with enumerated version numbers) is cleaned from the permanent spec library during this recording.
