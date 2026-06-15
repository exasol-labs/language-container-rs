# Plan: change-toolchain-edition-2024

## Summary

Consolidate the toolchain unification and an edition-2024 migration into a single plan for the `language-container-rs` workspace. Unify every Rust toolchain reference on **1.92** (bump the Docker builder stages `rust:1.91-bookworm` → `rust:1.92-bookworm`, raise the workspace pin `rust-toolchain.toml` `channel = "1.84"` → `"1.92"`, drop the now-redundant `cargo +1.91` / `cargo +1.92` prefixes in CI and scripts down to plain `cargo`), and migrate all **19** workspace crates from `edition = "2021"` to `edition = "2024"`. The one real code change is in the `exasol-udf-macros` proc-macro: it must emit `#[unsafe(no_mangle)]` instead of `#[no_mangle]` (an edition-2024 unsafe-attribute requirement). The Debian Bookworm base (`-bookworm`, glibc 2.36) is unchanged, so the SLC runtime ABI invariant is preserved.

**Success criterion (the north star):** a developer can run plain `cargo build` and `cargo test` with ZERO toolchain flags (`+version`) and ZERO edition juggling — one toolchain (1.92), one edition (2024), no `+N` overrides, no `default-members` split for toolchain reasons.

## Context

`1.92` is the minimum that clears the edition-2024 floor (edition 2024 stabilized in Rust 1.85) and matches the iceberg-main `rust-version = "1.92"` floor on the strata-rs side. The whole stack is unreleased, so raising the MSRV floor from `1.84` to `1.92` has no downstream cost. Scope is `language-container-rs` only — strata-rs is not touched.

The slim-image builder stage and every CI/local build invocation currently pin Rust 1.91 (`FROM rust:1.91-bookworm`, `cargo +1.91 ...`). The image `rust:1.92-bookworm` exists on Docker Hub and is built on Debian Bookworm with glibc 2.36 — byte-for-byte the same runtime ABI surface as `rust:1.91-bookworm` — so the load-bearing SLC invariant (glibc 2.36, binary runs on the Debian Exasol host after BucketFS extraction) holds without further change.

The workspace pin in `rust-toolchain.toml` is `channel = "1.84"`. This was a **false floor**: the `connect-back` test UDFs already depend on arrow 58, whose `edition = "2024"` transitive crates Rust 1.84 cannot parse, so those crates were excluded from `default-members` and built explicitly with `cargo +1.91`. Meanwhile the CI/builder image always ran the newer toolchain. Raising the pin to `1.92` unifies all three toolchains (workspace artifact, CI, Docker builder) on one version. Once the pin is `1.92`:
- plain `cargo` (no `+version`) parses and builds **every** workspace member, so the `default-members` exclusions for `connect-back-query`, `connect-back-insert`, `connect-back-crunch`, `connect-back-cluster-ip`, and `spike-connect` are obsolete and those crates move into `default-members`;
- `crates/it` stays out of `default-members`, but for a **different reason** — it needs a live Exasol Docker container to run meaningful tests, which is a runtime dependency, not a toolchain constraint;
- the explicit `cargo +1.91` / `cargo +1.92` overrides in `.github/workflows/ci.yml` and `scripts/ci-it-local.sh` become redundant and reduce to plain `cargo`.

**Edition 2024.** All 19 crates (8 under `crates/`, 11 under `test-udfs/`) are currently `edition = "2021"`; `crates/it` additionally carries `rust-version = "1.85"`. No `static mut` exists in the macros/runtime/client. The migration bumps every `edition` field to `"2024"`, collapses the MSRV to a single source of truth (the `1.92` toolchain pin; the per-crate `rust-version = "1.85"` is dropped), and fixes the one edition-2024 code incompatibility: the proc-macro at `crates/exasol-udf-macros/src/lib.rs:195` emits a bare `#[no_mangle]`, which edition 2024 rejects. Because proc-macro-emitted tokens are interpreted in the **call-site crate's edition**, the bare `#[no_mangle]` would error once the test-udf crates become edition 2024. Emitting `#[unsafe(no_mangle)]` is the fix and is valid on Rust >= 1.82 in BOTH editions, so the change is unconditional. The `unsafe extern "C"` blocks at lib.rs:152, 177, 213 are already edition-2024-correct. There are no public `-> impl Trait` signatures in `exa-udf-runtime` / `exasol-udf-sdk`, so the RPIT lifetime-capture change has no surface; `cargo fix --edition` is still run per crate to catch any remaining mechanical migrations (`gen` keyword reservation, `unsafe_op_in_unsafe_fn`, etc.).

Out of scope (deliberately untouched):
- strata-rs and everything outside `language-container-rs`.
- The Docker builders still `rm rust-toolchain.toml` before building (the image supplies its own 1.92 toolchain); this `rm` is left in place so the build is independent of the pinned channel value. The comment that referenced "the pinned 1.84" is updated to "the pinned 1.92".
- The pre-existing divergence between the `container/slim-image` "Alpine builder compiles the binary against musl" scenario (which describes `FROM rust:alpine`) and the actual glibc-based `Dockerfile.alpine` is not addressed here.

## Design

No ADR section — this is a coordinated version-and-edition migration with no new interfaces and no behavioral change to the shipped binary. The two architecturally relevant choices (raising the MSRV pin + moving to edition 2024; emitting `#[unsafe(no_mangle)]`) are captured in the decision log (Decisions promoted to ADR).

## Implementation Tasks

### Toolchain unification (1.84/1.91 → 1.92)

1. Update `Dockerfile.alpine` line 2: `FROM rust:1.91-bookworm AS builder` → `FROM rust:1.92-bookworm AS builder`. Update the line-19 comment referencing the 1.84/1.85 floor to reflect the unified 1.92 toolchain.
2. Update `Dockerfile.debian`: line 2 `FROM rust:1.91-bookworm AS builder` → `FROM rust:1.92-bookworm AS builder`; line 18-19 comment `the image's own toolchain (1.91) ... the pinned 1.84.` → `the image's own toolchain (1.92) ... the pinned 1.92.`.
3. Bump `rust-toolchain.toml`: `channel = "1.84"` → `channel = "1.92"`. Leave `targets = ["x86_64-unknown-linux-musl"]` and `components = ["rustfmt", "clippy"]` unchanged.
4. Update `.github/workflows/ci.yml` — replace every `cargo +1.91` with plain `cargo` (lines 62, 70, 82, 128, 135, 165, 169, 206, 209), bump every `dtolnay/rust-toolchain@1.91` install step to `@1.92` (lines 36, 109, 149, 186) and every `--toolchain 1.91` component install to `--toolchain 1.92` (lines 44, 114), and collapse the "workspace pin (1.84) ... CI uses 1.91" comments (lines 33-44, 108-114, 148-149, 183-186) into a single statement that the workspace, builder, and CI all use 1.92.
5. Update `scripts/ci-it-local.sh` — replace `cargo +1.91` with plain `cargo` (line 63 build, line 75 test --no-run) and update the log string on line 62 from `cargo +1.91 build --release` to `cargo build --release`.
6. Simplify `Cargo.toml` `default-members` (lines 24-44): move `test-udfs/connect-back-query`, `test-udfs/connect-back-insert`, `test-udfs/connect-back-crunch`, `test-udfs/connect-back-cluster-ip`, and `test-udfs/spike-connect` into `default-members`; replace the multi-line `cargo +1.91`-referencing comment block with a 1-2 line comment stating `it` requires a live Exasol container and is built/tested explicitly (`cargo test -p it --features integration`). No `cargo +N` reference remains.

### Edition 2024 migration

7. Bump `edition = "2021"` → `edition = "2024"` in all 19 crate manifests (`crates/*/Cargo.toml`, `test-udfs/*/Cargo.toml`).
8. Collapse the MSRV to a single source of truth: remove `rust-version = "1.85"` from `crates/it/Cargo.toml` (the `1.92` toolchain pin in `rust-toolchain.toml` is now the only MSRV declaration).
9. Fix the proc-macro FFI attribute: change `crates/exasol-udf-macros/src/lib.rs:195` from `#[no_mangle]` to `#[unsafe(no_mangle)]`. Verify the macro emits no other bare `#[no_mangle]`, `#[export_name]`, or `#[link_section]`. Do NOT alter the `unsafe extern "C"` shim blocks (already edition-2024-correct). [expert]
10. Run `cargo fix --edition` per crate under the 1.92 toolchain to catch mechanical migrations; review the diff for `unsafe_op_in_unsafe_fn`, `gen` keyword reservation, and RPIT lifetime-capture changes (scan public `-> impl Trait` signatures in `exa-udf-runtime` / `exasol-udf-sdk` — none currently exist, but re-confirm), then apply only what `cargo fix` proposes. [expert]
11. Relax the `trybuild = "=1.0.99"` dev-dependency pin in `crates/exasol-udf-macros/Cargo.toml`: the pin exists solely because `>= 1.0.100` pulls edition-2024 crates the 1.84 toolchain could not parse; under 1.92 the pin can be loosened (e.g. `trybuild = "1"`) and the justifying comment removed/updated. Keep the trybuild compile-fail fixtures green.

### Verification

12. Verify the default toolchain: `rustup show` reports `1.92` active at the repo root, and plain `cargo build` (no `+N`) compiles all `default-members` including `connect-back-*` and `spike-connect` under edition 2024.
13. `cargo clippy --all-targets --all-features -- -D warnings` and `cargo fmt --check` pass under edition 2024.
14. Run the integration tests against the **live Exasol Docker container** (`cargo test -p it --features integration`); they MUST FAIL (not skip) if Docker is unavailable. The `exa-udf-runtime` `dispatch` test loads a real `libscalar_double.so` (built from a macro-using, now-edition-2024 crate) and drives it through the runtime — this is the load-bearing proof the `#[unsafe(no_mangle)]` change produces a `.so` that still exports and runs `__exa_udf_entry`.
15. Smoke-test base image resolution and the image build: `docker pull rust:1.92-bookworm` succeeds; `docker build -f Dockerfile.alpine -t slc-rs-slim:dev .` completes and produces `/exaudf/exaudfclient`.
16. Grep the repo for any remaining `1.91`, `1.84`, `1.85`, `cargo +`, or `edition = "2021"` references and confirm zero hits outside `_recorded/` / `_plans/` history.

## Parallelization

| Parallel Group | Tasks |
|----------------|-------|
| Group A | 1, 2, 3, 4, 5, 6 (toolchain file edits — independent) |
| Group B | 7, 8 (edition field + MSRV manifest edits — independent of A and of each other) |
| Group C | 9 (proc-macro fix — independent file) |
| Group D | 10, 11 (cargo fix review + trybuild pin — depend on 7, 9 applied) |
| Group E | 12, 13, 14, 15, 16 (verification — depends on A–D) |

Sequential dependencies:
- Groups A, B, C run concurrently.
- Group D depends on Group B (edition bumped) and Group C (macro fixed) so `cargo fix` runs against the migrated tree.
- Group E (verification) runs last; task 14's dispatch test depends on 9 (macro emits `#[unsafe(no_mangle)]`) and 7 (`scalar-double` is edition 2024) both applied.

## Dead Code Removal

| Type | Location | Reason |
|------|----------|--------|
| Comment block | `Cargo.toml` lines 24-27, 42-43 | The `default-members` exclusion rationale and inline `cargo +1.91` build hints describe the obsolete 1.84-floor split; replaced by a 1-2 line live-container note. |
| Toolchain override prefixes | `.github/workflows/ci.yml`, `scripts/ci-it-local.sh` | `cargo +1.91` / `+1.92` prefixes are redundant once `rust-toolchain.toml` pins 1.92. |
| `rust-version = "1.85"` | `crates/it/Cargo.toml` | Per-crate MSRV diverges from the single 1.92 toolchain source of truth; removed. |
| trybuild version pin + comment | `crates/exasol-udf-macros/Cargo.toml` | The `=1.0.99` pin and its justifying comment exist only because of the 1.84 parse limit; obsolete under 1.92. |

## Verification

### Scenario Coverage

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| Builder toolchain and glibc runtime (CHANGED) | Integration | `.github/workflows/ci.yml` (build-slc job) + `test-udfs`/`it` db-roundtrip suite | `docker build -f Dockerfile.alpine` step + Alpine db-roundtrip integration run |
| Rust toolchain is pinned (CHANGED) | Integration | `.github/workflows/ci.yml` (build/clippy/test jobs) | plain `cargo build` / `cargo test` step running under the 1.92 pin compiles every default member |
| Workspace default-members covers every offline-buildable crate (NEW) | Integration | `.github/workflows/ci.yml` (build job) | `cargo build` over `default-members` compiles `connect-back-*` and `spike-connect` without a `+N` override |
| Every workspace crate declares edition 2024 (NEW) | Integration | `.github/workflows/ci.yml` (build job) | plain `cargo build` compiles every default member under edition 2024 (all 19 manifests `edition = "2024"`) |
| exasol_udf macro generates the entry point and vtable (CHANGED) | Integration | `crates/exa-udf-runtime/tests/dispatch.rs` | `dispatch` loads `libscalar_double.so` (macro-generated, edition 2024) and drives the run cycle — proves the `#[unsafe(no_mangle)]` entry point exports and dispatches |

The slim-image build is exercised by the CI `docker build` step (proves `rust:1.92-bookworm` resolves and the builder stage compiles `exaudfclient`); the resulting image is registered against `exasol/docker-db:2026.latest` and runs the db-roundtrip integration scenarios, transitively proving the glibc-2.36 ABI invariant still holds. The toolchain-pin, default-members, and edition scenarios are proved by the CI build/clippy/test jobs running plain `cargo` (no `+N`) against the 1.92 pin and compiling every default member under edition 2024. The macro-attribute change is proved by `dispatch.rs` loading and running a macro-generated edition-2024 `.so` through the runtime's `__exa_udf_entry` loader.

### Manual Testing

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| workspace/bootstrap | `rustup show` (at repo root) | Active toolchain reported as `1.92`. |
| workspace/bootstrap | `cargo build` (no `+N`, at repo root) | Compiles all `default-members` including `connect-back-*` and `spike-connect` under edition 2024; exit 0. |
| workspace/bootstrap | `cargo test` (no `+N`) | Runs without a toolchain override; exit 0 (default members only; `it` not included). |
| workspace/bootstrap | `grep -rn 'edition = "2021"' crates test-udfs` | Zero matches. |
| sdk/udf-macro | `cargo test -p exa-udf-runtime --test dispatch` | `dispatch` loads macro-generated `libscalar_double.so` (edition 2024) and runs the cycle; exit 0. |
| sdk/udf-macro | `grep -rn 'no_mangle' crates/exasol-udf-macros/src` | Only `#[unsafe(no_mangle)]`; no bare `#[no_mangle]`. |
| container/slim-image | `docker pull rust:1.92-bookworm` | Image pulls successfully (tag resolves on Docker Hub). |
| container/slim-image | `docker build -f Dockerfile.alpine -t slc-rs-slim:dev .` | Build completes; builder stage uses `rust:1.92-bookworm`; image contains `/exaudf/exaudfclient`. |
| container/slim-image | `docker build -f Dockerfile.debian -t slc-rs-slim-debian:dev .` | Build completes against `rust:1.92-bookworm`. |
| build tooling | `grep -rn '1\.91\|1\.84\|1\.85\|cargo +' Dockerfile.* .github/workflows/ci.yml scripts/ci-it-local.sh Cargo.toml rust-toolchain.toml crates/it/Cargo.toml` | Zero matches. |

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Build | `cargo build --release -p exaudfclient` | Exit 0 |
| Build all | `cargo build` | Exit 0 (all default-members, no `+N`, edition 2024) |
| Test | `cargo test` | 0 failures |
| Integration | `cargo test -p it --features integration` | 0 failures against live Exasol Docker; FAILS (not skips) if Docker unavailable |
| Lint | `cargo fmt --check && cargo clippy --all-targets --all-features -- -D warnings` | 0 errors/warnings |
| Format | `cargo fmt` | No changes |
| Image | `docker build -f Dockerfile.alpine -t slc-rs-slim:dev .` | Exit 0, `/exaudf/exaudfclient` present |

## Recording Notes — Spec-Library Cleanup on Record

This is a **spec-delta-free plan**: it carries `plan.md` and `decision-log.md` only, with no spec deltas under `specs/_plans/change-toolchain-edition-2024/`. The reason is a deliberate spec-philosophy decision (see decision-log entry [10]): **specs describe developer-observable behavior, not implementation details.** Version numbers, toolchain channels, Docker image tags, attribute spellings, edition values, and Cargo manifest fields are implementation details and do not belong in scenario specs. The work this plan performs (toolchain unification on a single version, edition migration, the `#[unsafe(no_mangle)]` attribute change) is entirely implementation detail at this granularity, so it generates no behavioral spec delta.

When this plan is recorded via `/speq:record`, the recorder MUST also clean the permanent spec library of implementation-detail leakage introduced in prior plans. Specifically:

**`specs/workspace/bootstrap/spec.md`** — the scenario "Rust toolchain is pinned" contains `channel = "1.84"` and `rustup show ... stable-1.84`. Remove this scenario entirely (it encodes a specific version number, not a behavior). The remaining scenarios in that spec are not affected.

**`specs/workspace/bootstrap/spec.md`** — the scenario "Workspace initialises with `[workspace.dependencies]`" lists specific version numbers (`zmq = "0.10"`, `arrow = "58"`, etc.). Strip the version literals; keep the scenario as "workspace dependencies are centralized in `[workspace.dependencies]`" without enumerating versions.

**`specs/design.md`** — this document contains version numbers, Dockerfile snippets (`FROM rust:1.84-bookworm`), and toolchain-specific text throughout. `design.md` is an architecture document (not a scenario spec), so it is out of scope for the speq spec system and should not be treated as a spec. The recorder should note it as a separate documentation cleanup, not a spec issue.

The recording-phase cleanup is intentionally separate from the implementation: it touches spec files, not code, and must pass `speq feature validate` after editing.
