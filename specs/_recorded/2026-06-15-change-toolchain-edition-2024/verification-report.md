# Verification Report: change-toolchain-edition-2024

## Bottom line

**PASS.** The `language-container-rs` workspace is unified on Rust **1.92** and migrated to **edition 2024**. A developer runs plain `cargo build` / `cargo test` with zero `+N` overrides and zero edition juggling. All implementation tasks (1–11), all code-review fixes (F1–F6, G1–G3), and all verification tasks (12–16 + the dispatch proof) are complete. The live Exasol integration suite (11/11 scenarios) and the slim-image build both pass.

## Checklist results

| Step | Command | Result |
|------|---------|--------|
| Toolchain | `rustup show` | `1.92` active via `rust-toolchain.toml` pin ✓ |
| Build | `cargo build` (no `+N`) | Exit 0 — all default-members incl `connect-back-*` + `spike-connect`, edition 2024 ✓ |
| Lint | `cargo clippy --all-targets --all-features -- -D warnings` | Exit 0, zero warnings ✓ |
| Format | `cargo fmt --check` | Exit 0, no diffs ✓ |
| Macro proof | `cargo test -p exa-udf-runtime --test dispatch` | 2 passed — loads macro-generated edition-2024 `.so`, drives `__exa_udf_entry` ✓ |
| Integration | `scripts/ci-it-local.sh` (live Exasol, `DB_MEM='4 GiB' MEM=12g SHM=2g`) | `db_roundtrip_all_scenarios`: 11/11 scenarios pass ✓ |
| Image | `docker pull rust:1.92-bookworm` + `docker build -f Dockerfile.alpine` | Builds; `/exaudf/exaudfclient` present (10 MB) ✓ |
| Stale-ref sweep | grep `1.91`/`1.84`/`1.85`/`cargo +`/`edition = "2021"` in impl files | Zero hits ✓ |

## Scenario coverage

| Scenario | Proof |
|----------|-------|
| Builder toolchain + glibc runtime (CHANGED) | `docker build -f Dockerfile.alpine` uses `rust:1.92-bookworm` (Debian Bookworm / glibc 2.36 unchanged); produces a working `exaudfclient` |
| Rust toolchain pinned (CHANGED) | plain `cargo build`/`cargo test` run under the 1.92 pin, no `+N` |
| default-members covers every offline-buildable crate (NEW) | `cargo build` compiles `connect-back-*` + `spike-connect` without `+N` |
| Every crate is edition 2024 (NEW) | plain `cargo build` compiles all 19 manifests at `edition = "2024"`; zero `edition = "2021"` |
| Macro emits entry point + vtable (CHANGED) | `dispatch.rs` loads `libscalar_double.so` (macro-generated, edition 2024) and runs the cycle; live `it` suite runs scalar/set/json/single-call/connect-back UDFs end-to-end |

## Notes / deltas beyond the literal task list

- **Additional edition-2024 mechanical fixes** were required and applied (canonical, no behavior change): `unsafe { set_var/remove_var }` in `exa-proto/build.rs`, `exaudfclient/src/main.rs`, and `it/src/lib.rs` test code; `#[unsafe(no_mangle)]` + `unsafe { }` bodies in `single-call-fixture`; `unsafe extern "C"` blocks in `exasol-udf-macros` (lib.rs malloc + vs_adapter test) and `exasol-udf-sdk/src/abi.rs` test.
- **clippy `collapsible_if`** (newly firing under 1.92 with stable let-chains) collapsed at two sites in `cargo-exaudf/src/build.rs`.
- **Code-review fixes** removed stale `--exclude connect-back-query/insert` from CI (those crates have no tests; only `it` needs a container), bumped README/docs MSRV `1.84+ → 1.92+`, rewrote `docs/cargo-ecosystem.md` (dropped `cargo +1.91` / 1.84-pin rationale), set the `cargo exaudf new` scaffold template to `edition = "2024"`, and de-tautologised the `Dockerfile.debian` comment.

## Recording follow-up (for `/speq:record`)

This is a spec-delta-free plan (`plan.md` + `decision-log.md` only). On record, also clean implementation-detail leakage from the permanent spec library per plan.md §"Recording Notes": remove the version-pinned "Rust toolchain is pinned" scenario and strip version literals from the `[workspace.dependencies]` scenario in `specs/workspace/bootstrap/spec.md`; note `specs/design.md` version/Dockerfile cleanup as documentation (not spec). Must pass `speq feature validate` after editing.
