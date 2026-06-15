# Decision Log: change-toolchain-edition-2024

Date: 2026-06-15

## Interview

This plan consolidates and renames the former `change-rust-toolchain-1-92` plan into a single upgrade that unifies the toolchain on 1.92 AND moves the whole `language-container-rs` workspace to edition 2024. Scope stays `language-container-rs` only — strata-rs is untouched. The north star is developer experience: plain `cargo build` / `cargo test`, one toolchain, one edition, no `+N` overrides, no `default-members` split.

- **Q (implicit): Why does the 1.84 pin exist?** A: 1.84 is the declared MSRV for the SDK core crates (all `edition = "2021"`, compile under 1.84). The `default-members` split exists because arrow 58 transitively pulls in `edition = "2024"` crates that Rust 1.84 cannot *parse* (a parser failure, not a version conflict); the explicit `cargo +1.91` overrides bypass the toolchain file for those invocations.
- **Q: Should the pin be bumped to 1.92 as part of this plan?** A: Yes. The user wants the workspace, CI, and builder toolchains unified on 1.92 so the `default-members` split and the `cargo +N` overrides can be removed.
- **Q: Why 1.92 specifically and not latest stable?** A: 1.92 is the minimum that clears the edition-2024 floor (edition 2024 stabilized in 1.85) and matches the iceberg-main `rust-version = "1.92"` floor on the strata-rs side. The whole stack is unreleased, so raising the MSRV floor from 1.84 to 1.92 has no downstream cost. Latest stable was explicitly NOT chosen — 1.92 is the deliberate, justified pin.
- **Q: Should the `cargo +1.92` prefixes be kept after the bump?** A: No — once `rust-toolchain.toml` pins 1.92, plain `cargo` already uses 1.92, so the `+N` prefix is dropped entirely. The net change in CI/scripts is `cargo +1.91` → `cargo`.
- **Q: Which crates move into `default-members`?** A: `connect-back-query`, `connect-back-insert`, `connect-back-crunch`, `connect-back-cluster-ip`, `spike-connect` (pure compilation, no live dependencies). `crates/it` stays excluded — but the reason changes from "toolchain can't parse it" to "needs a live Exasol Docker container".
- **Q: Move the workspace to edition 2024?** A: Yes — all 19 crates move from `edition = "2021"` to `edition = "2024"`. One toolchain + one edition is the pure-DX goal.
- **Q: How to handle the MSRV now that crates/it carries `rust-version = "1.85"`?** A: Keep it simple — one source of truth. The `1.92` toolchain pin is the only MSRV declaration; the per-crate `rust-version = "1.85"` on `crates/it` is removed.
- **Q: What real code changes does edition 2024 require?** A: One. The `exasol-udf-macros` proc-macro emits a bare `#[no_mangle]`; edition 2024 makes `#[no_mangle]`/`#[export_name]`/`#[link_section]` unsafe attributes. The macro must emit `#[unsafe(no_mangle)]`. Verified: only one bare `#[no_mangle]` exists (lib.rs:195); the `unsafe extern "C"` blocks are already correct; no `static mut` exists; no public `-> impl Trait` signatures exist in runtime/sdk (so the RPIT change has no surface). `cargo fix --edition` is still run per crate for any remaining mechanical migrations.

## Design Decisions

### [1] Target the `container/slim-image` feature for the builder-image bump

- **Decision:** Author the builder-image delta against `container/slim-image` ("Builder toolchain and glibc runtime" scenario), the only scenario whose GIVEN names the builder image version (`rust:1.91-bookworm`).
- **Alternatives:** Put the builder-image change on `workspace/bootstrap`. Rejected: the builder image is a slim-image concern; the toolchain *pin* is the workspace-bootstrap concern (see Decision [2]).
- **Rationale:** Each spec'd behavior is changed where it is actually asserted.
- **Promotes to ADR:** no

### [2] Raise the `rust-toolchain.toml` MSRV pin from 1.84 to 1.92

- **Decision:** Change `rust-toolchain.toml` `channel = "1.84"` → `"1.92"` (targets and components unchanged), and author a `workspace/bootstrap` "Rust toolchain is pinned" delta to match.
- **Supersedes:** The earlier revision of this plan decided to *leave* the pin at 1.84 as a deliberate, decoupled workspace-artifact baseline. The user has since confirmed that the 1.84 floor was a **false floor** and explicitly asked to bump it. This decision reverses that earlier one.
- **Alternatives:** Keep 1.84 and retain the `default-members` split + `cargo +N` overrides (rejected: re-litigated and overturned by the user); delete the pin entirely (rejected: a pin is still wanted so `rustup` selects 1.92 deterministically and the Docker `rm rust-toolchain.toml` fallback to the image toolchain remains a clean no-op).
- **Rationale:** The `connect-back` feature users already required Rust >= 1.85 (arrow 58 / edition 2024 transitive deps), and the CI/builder image always used 1.91. Raising to 1.92 unifies the workspace, CI, and builder toolchains on one version, eliminating the `default-members` split and all `cargo +N` overrides.
- **Promotes to ADR:** yes

### [3] Collapse the `default-members` split

- **Decision:** Move `connect-back-query`, `connect-back-insert`, `connect-back-crunch`, `connect-back-cluster-ip`, and `spike-connect` into `default-members`; keep `crates/it` out; replace the `cargo +1.91`-referencing comment block with a 1-2 line note that `it` needs a live Exasol container.
- **Alternatives:** Leave `default-members` as-is (rejected: the exclusions only existed because of the 1.84 parser limit, now removed); also move `it` in (rejected: `it` requires a live Docker Exasol container to run meaningful tests, so including it in default builds would surprise contributors with container-dependent failures).
- **Rationale:** After the 1.92 pin, the only remaining reason to exclude a crate from default builds is a *runtime* dependency (live container), not a toolchain parse failure. The comment is rewritten to capture that distinction.
- **Promotes to ADR:** no

### [4] Drop all `cargo +N` override prefixes in CI and scripts

- **Decision:** Replace every `cargo +1.91` in `.github/workflows/ci.yml` and `scripts/ci-it-local.sh` with plain `cargo`, and bump the `dtolnay/rust-toolchain@1.91` / `--toolchain 1.91` install steps to `1.92`.
- **Alternatives:** Change `+1.91` → `+1.92` and keep the override (rejected: redundant once the pin is 1.92, and the user explicitly asked to drop the prefix); keep installing 1.91 alongside (rejected: leaves a stale toolchain that diverges from the pin).
- **Rationale:** With `rust-toolchain.toml` at 1.92, plain `cargo` resolves to 1.92, so the override is dead noise. Removing it is the combined effect of the original 1.91→1.92 bump plus this extension.
- **Promotes to ADR:** no

### [5] No scenario change for CI / scripts beyond the toolchain assertion

- **Decision:** Update the CI workflow and `scripts/ci-it-local.sh` via implementation tasks only; the spec change is limited to the toolchain-pin and Cargo.toml `workspace/bootstrap` scenarios plus the slim-image builder scenario.
- **Alternatives:** Add scenarios asserting CI uses plain `cargo`. Rejected: CI invocation strings are build tooling, not spec'd product behavior; the bootstrap and slim-image scenarios already prove the toolchain works end-to-end.
- **Rationale:** Keeps the spec surface focused on observable behavior (the pin value, the buildable default members, the builder image).
- **Promotes to ADR:** no

### [6] glibc 2.36 invariant made explicit in the slim-image scenario

- **Decision:** Keep the clause in the changed slim-image scenario asserting the builder image resolves to a Debian Bookworm base providing glibc 2.36, preserving the SLC runtime ABI invariant.
- **Alternatives:** Change only the version string. Rejected: the version string alone does not capture *why* `-bookworm` is load-bearing.
- **Rationale:** Guards future bumps against silently moving off Bookworm.
- **Promotes to ADR:** no

### [7] Move the whole workspace to edition 2024; pin the toolchain at 1.92

- **Decision:** Migrate all 19 crate manifests from `edition = "2021"` to `edition = "2024"`, and pin `rust-toolchain.toml` at `1.92` — the minimum that clears the edition-2024 floor (edition 2024 stabilized in 1.85) and matches the iceberg-main `rust-version = "1.92"` floor on the strata-rs side. The MSRV becomes a single source of truth (the toolchain pin); the per-crate `rust-version = "1.85"` on `crates/it` is removed.
- **Alternatives:** Pin at latest stable (rejected: 1.92 is the justified minimum that satisfies both the edition-2024 floor and the strata-rs iceberg floor; pinning higher buys nothing and drifts from the strata-rs side). Keep mixed editions (rejected: defeats the one-edition DX goal). Keep a per-crate `rust-version` (rejected: two MSRV sources to keep in sync; the toolchain pin is the single source).
- **Rationale:** The whole stack is unreleased, so raising the MSRV floor from 1.84 to 1.92 carries no downstream cost. One toolchain + one edition eliminates the dual-toolchain / `default-members` complexity — a pure DX win. The success criterion is that a developer runs plain `cargo build` / `cargo test` with zero `+N` flags and zero edition juggling.
- **Promotes to ADR:** yes

### [8] Proc-macro emits `#[unsafe(no_mangle)]` for the generated UDF entry point

- **Decision:** Change the `exasol-udf-macros` proc-macro to emit `#[unsafe(no_mangle)]` instead of `#[no_mangle]` for the generated `__exa_udf_entry` (lib.rs:195). Verify no other bare `#[no_mangle]`/`#[export_name]`/`#[link_section]` is emitted; leave the already-correct `unsafe extern "C"` shim blocks unchanged.
- **Alternatives:** Gate the attribute on the call-site edition (rejected: impossible/fragile — proc-macros cannot reliably know the consumer edition, and it is unnecessary). Keep `#[no_mangle]` and hold the test-udf crates at edition 2021 (rejected: defeats the one-edition goal, since the macro output is interpreted in the call-site crate's edition and would error there).
- **Rationale:** Proc-macro-emitted tokens are interpreted in the call-site crate's edition; edition 2024 makes `#[no_mangle]` an unsafe attribute, so once a test-udf crate is edition 2024 the bare form errors at its call site. `#[unsafe(no_mangle)]` is valid on Rust >= 1.82 in BOTH editions 2021 and 2024, so the change is unconditional and safe regardless of any consumer's edition. The `dispatch.rs` test loads a real macro-generated `.so` and runs it through the runtime, proving the entry point still exports and dispatches.
- **Promotes to ADR:** yes

### [9] Relax the trybuild `=1.0.99` dev-dependency pin

- **Decision:** Loosen `trybuild = "=1.0.99"` in `crates/exasol-udf-macros/Cargo.toml` (e.g. to `"1"`) and remove/update the justifying comment, since the pin existed only because `>= 1.0.100` pulled edition-2024 crates the 1.84 toolchain could not parse.
- **Alternatives:** Keep the pin (rejected: it is dead weight under 1.92 and obscures why it existed).
- **Rationale:** The 1.84 parse limitation is gone; keeping a hard `=` pin invites stale-dependency drift. Compile-fail fixtures must stay green after the bump.
- **Promotes to ADR:** no

### [10] Spec deltas removed — specs describe behavior, not implementation detail

- **Decision:** Remove all three spec delta files from this plan (`container/slim-image`, `sdk/udf-macro`, `workspace/bootstrap`), making this a spec-delta-free plan (`plan.md` + `decision-log.md` only). Specs describe developer-observable behavior, not implementation details: version numbers, toolchain channels, Docker image tags, attribute spellings (`#[no_mangle]` vs `#[unsafe(no_mangle)]`), edition values, and Cargo manifest fields are all implementation detail and do not belong in scenario specs.
- **Supersedes:** Decisions [1], [2], [5], [6] (and the spec-authoring portions of [3], [7], [8]) committed deltas that encoded version strings (`rust:1.91-bookworm` → `rust:1.92-bookworm`, `channel = "1.84"` → `"1.92"`), edition literals, and the macro attribute spelling directly into scenario specs. Those deltas are deleted; the decisions remain as historical record but their spec-delta outputs are withdrawn. The implementation tasks are unaffected — they describe what to do, independent of whether the change produces a spec delta.
- **Alternatives:** Keep the deltas (rejected: they encode implementation detail the spec system should never hold; they would re-introduce exactly the version-number leakage this philosophy forbids). Rewrite the deltas to describe behavior abstractly (rejected: at this granularity the change is pure implementation detail — toolchain version, edition value, FFI attribute spelling — with no observable behavioral surface to assert, so there is nothing behavioral to spec).
- **Rationale:** A plan that carries only a decision log and implementation tasks is legitimate when the work has no observable behavioral delta. The toolchain/edition migration preserves all shipped behavior (the SLC runtime ABI invariant is explicitly held); the only externally observable effect — a developer running plain `cargo build`/`cargo test` with no `+N` flags — is a build-tooling/DX property, not a product behavior spec asserts.
- **Promotes to ADR:** yes

## Review Findings

Code review (Pyramid-grouped) surfaced 7 findings; all addressed:

- **Stale CI excludes (blocker):** `.github/workflows/ci.yml` still `--exclude`d `connect-back-query`/`connect-back-insert` from the three `--workspace` build/clippy/test steps. Verified those crates (and crunch/cluster-ip/spike-connect) have NO tests — only `it` needs a live container. Removed the two excludes (kept `--exclude it`) and corrected the comment.
- **Stale MSRV in user docs (major):** README badge + prereq and `docs/writing-a-udf.md` said `Rust 1.84+`; `docs/cargo-ecosystem.md` carried the obsolete `cargo +1.91` / 1.84-pin / "connect-back excluded" rationale. Updated to `1.92+`, default-members reality, plain `cargo`.
- **Scaffold template (major):** `cargo-exaudf/src/new.rs` generated new crates at `edition = "2021"` → set to `"2024"`.
- **Tautological comment (nit):** `Dockerfile.debian` "...rather than the pinned 1.92" → matched `Dockerfile.alpine` ("no version split").
- The edition-2024 `unsafe` migrations were reviewed as minimal/canonical (no over-broad `unsafe`, SAFETY comments present).

Verification-phase additions (gates that `cargo build` alone did not exercise): clippy `collapsible_if` collapsed (let-chains) in `cargo-exaudf/src/build.rs`; workspace `cargo fmt` applied (edition-2024 style); further `unsafe extern "C"` / `unsafe { set_var }` fixes in `exasol-udf-sdk` and `it` test code surfaced by `--all-targets` / the `it` test binary.

Outcome: `cargo build`, `clippy --all-targets --all-features -D warnings`, `cargo fmt --check`, the `dispatch` macro proof, the 11-scenario live Exasol integration suite, and the `Dockerfile.alpine` image build all pass under 1.92 / edition 2024.
