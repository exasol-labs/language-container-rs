# Tasks: change-toolchain-edition-2024

## Phase 2: Implementation (Group A — toolchain file edits)
- [x] 2.1 Dockerfile.alpine: `rust:1.91-bookworm` → `rust:1.92-bookworm`; update floor comment
- [x] 2.2 Dockerfile.debian: builder image → `rust:1.92-bookworm`; comment 1.91/1.84 → 1.92
- [x] 2.3 rust-toolchain.toml: `channel = "1.84"` → `"1.92"` (targets/components unchanged)
- [x] 2.4 .github/workflows/ci.yml: `cargo +1.91` → `cargo`; `@1.91`/`--toolchain 1.91` → 1.92; collapse comments
- [x] 2.5 scripts/ci-it-local.sh: `cargo +1.91` → `cargo`; update log string
- [x] 2.6 Cargo.toml: move connect-back-* + spike-connect into default-members; rewrite comment (it = live container)

## Phase 2: Implementation (Group B — edition + MSRV manifest edits)
- [x] 2.7 Bump `edition = "2021"` → `"2024"` in all 19 crate manifests (crates/*, test-udfs/*)
- [x] 2.8 Remove `rust-version = "1.85"` from crates/it/Cargo.toml

## Phase 2: Implementation (Group C — proc-macro fix)
- [x] 2.9 exasol-udf-macros/src/lib.rs:195 `#[no_mangle]` → `#[unsafe(no_mangle)]`; verify no other bare FFI attrs [expert]

## Phase 2: Implementation (Group D — cargo fix + trybuild; depends on B, C)
- [x] 2.10 Run `cargo fix --edition` per crate under 1.92; review/apply only proposed mechanical migrations [expert]
- [x] 2.11 Relax `trybuild = "=1.0.99"` → `"1"` in exasol-udf-macros/Cargo.toml; update comment; keep fixtures green

## Phase 2.5: Code-review fixes (Group F — depends on A–D)
- [x] F1 ci.yml: drop stale `--exclude connect-back-query`/`--exclude connect-back-insert` (3 steps); comment → only `it` needs containers
- [x] F2 README.md: `Rust 1.84+` → `1.92+` (badge line 5 + prereq line 31)
- [x] F3 docs/writing-a-udf.md:9 `Rust 1.84+` → `1.92+`
- [x] F4 docs/cargo-ecosystem.md: connect-back-* now in default-members (plain `cargo build`); `it` needs Docker (`cargo test -p it --features integration`); drop `cargo +1.91`; pin 1.84 → 1.92
- [x] F5 crates/cargo-exaudf/src/new.rs:37 scaffold `edition = "2021"` → `"2024"`
- [x] F6 Dockerfile.debian: fix tautological comment (match Dockerfile.alpine "no version split")

## Phase 2.5: Lint/format gate fixes (Group G)
- [x] G1 cargo-exaudf/src/build.rs: collapse 2 `collapsible_if` (let-chains, edition 2024)
- [x] G2 exasol-udf-sdk/src/abi.rs: `extern "C"` test blocks → `unsafe extern "C"`
- [x] G3 `cargo fmt` workspace-wide (edition-2024 style); `cargo fmt --check` clean

## Phase 3: Verification (Group E — depends on A–G)
- [x] 3.12 `rustup show` = 1.92 (active via pin); plain `cargo build` compiles all default-members edition 2024 — PASS
- [x] 3.13 `cargo clippy --all-targets --all-features -- -D warnings` exit 0; `cargo fmt --check` exit 0 — PASS
- [x] 3.14 `cargo test -p it --features integration` against live Exasol Docker — 11/11 scenarios PASS (also fixed edition-2024 `unsafe` set_var/remove_var in it test code)
- [x] 3.15 `docker pull rust:1.92-bookworm` + `docker build -f Dockerfile.alpine` → `/exaudf/exaudfclient` present — PASS
- [x] 3.16 Grep repo: zero `1.91`/`1.84`/`1.85`/`cargo +`/`edition = "2021"` in impl files — PASS
- [x] 3.x dispatch.rs (macro `#[unsafe(no_mangle)]` load-bearing proof): 2 passed — PASS
