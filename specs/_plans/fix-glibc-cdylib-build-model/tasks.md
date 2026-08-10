# Tasks: fix-glibc-cdylib-build-model

## Group A — CLI build model (code)

- [x] 1.1 Rewrite `crates/cargo-exasol-udf/tests/build.rs`. Rename `build_produces_musl_so` → `build_produces_host_cdylib` and remove its `#[ignore]`. Before invoking build, inject a `[patch.crates-io]` block into the scaffolded crate's `Cargo.toml` pointing `exasol-udf-sdk` and `exasol-udf-macros` at the in-repo crate paths (resolve the workspace root from `env!("CARGO_MANIFEST_DIR")` + `/../..`, then `crates/exasol-udf-sdk` and `crates/exasol-udf-macros`), so the scaffold builds against the LOCAL workspace SDK with no dependency on a published crates.io version. Assert stdout prints `target/release/lib<crate>.so` (no musl triple), the produced `.so` exists, and it exports at least one `__exa_udf_entry_<NAME>` symbol. Add `build_honors_target_override` WITHOUT `#[ignore]`: read the host target triple from `rustc -vV` (the `host:` line — installed by definition, `x86_64-unknown-linux-gnu` on CI), inject the same `[patch.crates-io]` block, run `cargo exaudf build --target <host-triple>`, and assert the `target/<host-triple>/release/lib<crate>.so` path. Delete `build_installs_missing_target`; keep `build_fails_on_missing_cargo_toml`. The two build-invoking tests need only `cargo` (no `rustup` target install), so they run unconditionally in CI, not behind a skip. Write these as the failing tests first. [expert]
- [x] 1.2 Repoint `crates/cargo-exasol-udf/src/build.rs`: delete the `MUSL_TARGET` const and `ensure_musl_target()` (and its call); default `build` to `cargo build --release` with no `--target`, artifact path `target/release/lib<crate>.so`; add `--target <triple>` arg parsing that invokes `cargo build --release --target <triple>` and restores the per-target path join `target/<triple>/release/`; keep the post-build entry-symbol verification and the sidecar path. Make 1.1's tests pass. [expert]
- [x] 1.3 Repoint `crates/cargo-exasol-udf/src/new.rs:43-44` — change the scaffolded `Cargo.toml` version pins `exasol-udf-sdk = { version = "0.1", features = [] }` and `exasol-udf-macros = { version = "0.1" }` to the current SDK major.minor line `"0.21"` (tracks `[workspace.package].version = 0.21.3`, per the project rule that the scaffold pin tracks the published SDK version). Required for 1.1's un-ignored `build_produces_host_cdylib` gate to resolve and compile the scaffold against the current SDK; the `__exa_udf_entry_<NAME>` named-entry ABI the build verifies postdates 0.1. [expert]

## Group B — Config / toolchain / artifact removal

- [x] 2.1 `crates/cargo-exasol-udf/Cargo.toml:8` — change the description from `...build (static musl .so)...` to glibc-cdylib wording (e.g. `scaffold, build (glibc cdylib .so), and validate`).
- [x] 2.2 `rust-toolchain.toml` — remove the `targets = ["x86_64-unknown-linux-musl"]` line; keep `channel` and `components`.
- [x] 2.3 `.cargo/config.toml` — remove the `[target.x86_64-unknown-linux-musl]` / `linker = "musl-gcc"` stanza; keep (and, if needed, lightly adjust) the header comment documenting the glibc `cargo build --release -p <crate>` path as canonical.
- [x] 2.4 Remove `targets/x86_64-unknown-linux-musl-dylib.json` and the `COPY targets/ ./targets/` line in `Dockerfile.alpine` (line ~18); confirm the Alpine build still succeeds (it already `rm`s `rust-toolchain.toml` and builds glibc with no `--target`). NOTE: overlaps PR #79 P7 task 7.1 which removes the same file — since #80 merges first, #79 drops the redundant deletion on rebase.

## Group C — Docs + architecture.md reconciliation

- [x] 3.1 `README.md` — L61/L68 artifact paths `target/x86_64-unknown-linux-musl/release/libdouble.so` → `target/release/libdouble.so`; L119 table cell `Scaffold, build (static musl .so), validate` → glibc-cdylib wording.
- [x] 3.2 `docs/writing-a-udf.md` — Prerequisites (L9-12): drop the `rustup target add x86_64-unknown-linux-musl` step (host glibc build needs no musl target). §13 Build and deploy (L597-605): replace `Cross-compile to a musl .so` + artifact path `target/x86_64-unknown-linux-musl/release/libmy_udf.so` with a plain host `cargo exasol-udf build` producing `target/release/libmy_udf.so`. L614: replace the `equivalent to cargo build --target x86_64-unknown-linux-musl --release` line with the glibc default + `--target` override wording. SCOPE: build-model wording only — leave any arch/Personal-specific note that PR #79 adds (Finding B) untouched.
- [x] 3.3 `docs/cargo-ecosystem.md` — L84 subcommand row `Cross-compile to x86_64-unknown-linux-musl .so` → `Build a glibc cdylib .so (host default), or --target <triple>`; L87 `selects the musl target ... equivalent to cargo build --target x86_64-unknown-linux-musl --release` → glibc default + `--target` override wording.
- [x] 3.4 `specs/architecture.md` — L41 `user libudf.so (static musl cdylib)` → `user libudf.so (glibc cdylib)`; L91 project-structure comment `CLI — scaffold + build musl .so` → `CLI — scaffold + build the cdylib .so`. Direct edit (root-level reference doc, same treatment as `mission.md`; not delta-merged).

NOTE: `docs/installation.md` needs no build-model edit — on current `main` it carries no musl/static/cdylib/glibc or support-matrix framing, and its only build mention is build-model-neutral. Its support-matrix wording is PR #79's to add and keep consistent; #80 does not touch it.

## Optional cleanup

- [x] 4.1 `crates/exasol-udf-sdk/build.rs:8` — remove the stale comment referencing a `cargo -Z build-std` invocation that does not exist. (Low priority; skip if it risks churn with PR #79.)

## Phase 4: Review Fixes

- [x] 4.2 `crates/cargo-exasol-udf/src/build.rs` — immediately after the `println!("{}", so_path.display());` in `run`, add a guard that returns `Err` when `!so_path.exists()` (e.g. `format!("cargo build succeeded but no artifact was produced at '{}'", so_path.display())`); then drop the now-redundant `if so_path.exists()` conditions guarding the entry-symbol verification and sidecar emit so both run unconditionally on the guaranteed-present path.
- [x] 4.3 `docs/cargo-ecosystem.md` line ~87 — delete the false "sets `CARGO_TARGET_DIR`," clause so the sentence no longer claims the CLI sets that env var.
