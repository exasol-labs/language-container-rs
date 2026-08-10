# Code Review Findings: fix-glibc-cdylib-build-model

## Summary
- Files reviewed: 14 (3 code: build.rs, new.rs, tests/build.rs; 1 build script comment: exasol-udf-sdk/build.rs; Cargo.toml; 4 config/artifact removals; 5 docs/spec wording)
- Total findings: 2 (standard: 2, expert: 0)

Scope notes verified clean (no finding):
- No leftover musl *build-model* reference in any in-scope file. Remaining `musl` hits are the intentional negative assertion in `tests/build.rs:114-115` and out-of-scope files (`specs/tools/*`, `specs/examples/*`, `specs/_decision/*`, the Alpine client-binary comment) — none in the changed-files list.
- `clippy -p cargo-exasol-udf --all-targets` is clean: no dead code, unused import, or suppressed warning from the `MUSL_TARGET`/`ensure_musl_target` deletion or the test rewrite.
- No orphaned `build_tests.rs`; `build.rs` no longer references `rustup` or `MUSL_TARGET`.
- `new.rs` pin `0.21` matches the workspace SDK line (`0.21.3`, `^0.21` satisfied).
- Config/artifact removals (`rust-toolchain.toml`, `.cargo/config.toml`, `Dockerfile.alpine`, deleted `targets/*.json`) and the SDK `build.rs` comment rewrite are correct and coherent.
- Accepted items (a) `--target` missing-value returns `Err`, unit-test omitted; (b) `about.toml` musl pin deferred to #79; (c) staged JSON deletion — not re-flagged.

## Standard fixes

### crates/cargo-exasol-udf/src/build.rs

#### [SWALLOWED_ERROR] `build` reports success when cargo produced no artifact
- Location: lines 45-64 (`run`)
- Issue: After a successful `cargo build`, `run` prints `so_path` unconditionally (line 45), then guards both the entry-symbol verification (line 47) and the sidecar emit (line 58) with `if so_path.exists()`. When `cargo build` succeeds but no `.so` exists at the computed path — reachable now that glibc builds actually succeed, e.g. a crate with a `[lib] name = "..."` override or a non-`cdylib` crate-type so `lib<crate_name>.so` never matches — both blocks are skipped and the function returns `Ok(())`. The command prints a path to a nonexistent file and exits 0, and the doc-comment promise "verify the produced artifact exports named entry points" is silently not kept. This is the exact silent-regression the plan's build-gating test aims to prevent.
- Fix: In `crates/cargo-exasol-udf/src/build.rs`, immediately after the `println!("{}", so_path.display());` on line 45, add a guard that returns `Err` when the artifact is absent, e.g. `if !so_path.exists() { return Err(format!("cargo build succeeded but no artifact was produced at '{}'", so_path.display())); }`. Then drop the now-redundant `if so_path.exists()` conditions on lines 47 and 58 so the entry-symbol verification and sidecar emit run unconditionally on the guaranteed-present path.

### docs/cargo-ecosystem.md

#### [OUTDATED_COMMENT] Doc claims the CLI sets `CARGO_TARGET_DIR`, which it never does
- Location: line 87
- Issue: The rewritten sentence still asserts "`cargo exasol-udf build` sets `CARGO_TARGET_DIR`, defaults to the host glibc target, and passes `--release`". Verified `CARGO_TARGET_DIR` is never set anywhere in `crates/cargo-exasol-udf/` (`grep -rn CARGO_TARGET_DIR` returns nothing); `run` only invokes `cargo build --release [--target <triple>]` with `current_dir(crate_dir)`. The clause is a pre-existing falsehood the implementer preserved while editing this exact sentence for the musl→glibc change.
- Fix: In `docs/cargo-ecosystem.md` line 87, delete the false "sets `CARGO_TARGET_DIR`," clause so the sentence reads that `cargo exasol-udf build` defaults to the host glibc target and passes `--release` — equivalent to `cargo build --release` (or `cargo build --target <triple> --release` with `--target`) — without remembering the flags.

## Expert fixes
[none]
