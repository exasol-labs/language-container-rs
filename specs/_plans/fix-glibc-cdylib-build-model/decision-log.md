# Decision Log: fix-glibc-cdylib-build-model

## Interview

**Q:** What should `cargo exasol-udf build` target?
**A:** Glibc default plus override. Repoint the CLI default to a host glibc cdylib (plain `cargo build --release`, artifact at `target/release/lib<crate>.so`), and keep `--target <triple>` as an optional override escape hatch. Changes `crates/cargo-exasol-udf/src/build.rs` and its tests. Yields one working end-to-end model.

**Q:** What is the scope for the musl toolchain/config artifacts?
**A:** Remove them in this plan. Drop the musl target from `rust-toolchain.toml`, the `[target.x86_64-unknown-linux-musl] linker = musl-gcc` stanza from `.cargo/config.toml`, and `targets/x86_64-unknown-linux-musl-dylib.json` (plus the `COPY targets/ ./targets/` line in `Dockerfile.alpine`). Coordinate with the arm64 plan's P7 task, which also plans the `targets/*.json` removal.

**Q:** PR #79 (`feat/add-arm64-support`) already edits the same `cargo-exaudf` spec, `build.rs`, docs, and config toward a host-arch *musl* model, and adds a musl `--target` override scenario that is itself unbuildable — how do #79 and this plan sequence?
**A:** Decision (A): this plan (#80) lands first and solely owns the glibc-dynamic-cdylib build model; author against current `main`. Do not amend PR #79. PR #79 declares `Depends on #80` and rebases after #80 merges, dropping the content #80 supersedes and renumbering its arm64 ADR 024→025.

## Design Decisions

### [1] Adopt glibc-dynamic cdylib as the single UDF artifact model; build defaults to host glibc with a --target override

- **Decision:** Every deployable UDF `.so` is a glibc-dynamic cdylib built by a plain host `cargo build --release` (artifact `target/release/lib<crate>.so`). `cargo exasol-udf build` uses that as its default and exposes an optional `--target <triple>` that restores the per-target path `target/<triple>/release/lib<crate>.so` for a native build on a host with that target installed. No `rustup target add` auto-install.
- **Alternatives:** Keep the "fully-static musl `.so`" model — rejected: a musl target defaults `crt-static` to true, so `rustc` emits no cdylib and errors `cannot produce cdylib ... does not support these crate types`; the musl `build` path cannot produce any artifact. Drop `--target` entirely — rejected: it is a cheap escape hatch for a native non-default-host build and costs nothing to keep.
- **Rationale:** glibc-dynamic cdylib is the only buildable model and matches the shipped SLC runtime (bundled glibc) and CI, which already builds fixtures this way (`cargo build --release -p <crate>`, `target/release/lib*.so`). A sensible default removes a required flag from the common path (design-philosophy: a config parameter is a decision the module declined to make).
- **Supersedes:** cargo-exaudf hides the musl target triple from authors
- **Promotes to ADR:** yes

### [2] Remove the dead musl toolchain and target-spec config in this plan

- **Decision:** Delete the musl entry from `rust-toolchain.toml`, the `[target.x86_64-unknown-linux-musl]` linker stanza from `.cargo/config.toml`, `targets/x86_64-unknown-linux-musl-dylib.json`, and the `COPY targets/ ./targets/` line in `Dockerfile.alpine`.
- **Alternatives:** Defer the config removal to PR #79's arm64 work — rejected: #79 depends on #80 and rebases after it, so #80 is the natural owner of the build-model cleanup; deferring would leave dead config referencing an unbuildable target.
- **Rationale:** Nothing feeds the target JSON to `rustc`; the sole reference is the Dockerfile `COPY` line. The musl target and linker stanza serve a build path that cannot produce a cdylib. Keeping them invites the assumption that they are load-bearing.
- **Promotes to ADR:** no

### [3] Un-ignore a build-invoking test to gate the build subcommand

- **Decision:** Rename and un-`#[ignore]` the default-build test so `cargo exasol-udf build` on a scaffolded crate is exercised in CI, asserting a loadable `target/release/lib<crate>.so`. Un-`#[ignore]` the `--target` override test too, using the host triple `x86_64-unknown-linux-gnu` as the concrete `--target` (installed by definition). Both gates require the scaffold to build against the current SDK: repoint the `new` scaffold's `exasol-udf-sdk`/`exasol-udf-macros` pins to the current major.minor line (task 1.3), and inject a `[patch.crates-io]` in the test pointing those crates at the in-repo paths, so the build resolves the local SDK with no crates.io dependency.
- **Alternatives:** Leave all three build tests `#[ignore]`d — rejected: that gap is exactly why a fully-broken `build` subcommand (musl cdylib → hard error) shipped undetected. Assume the glibc default needs no further change to run unignored — rejected: the scaffold's `"0.1"` pins neither resolve nor compile against the `0.21.x` SDK whose `__exa_udf_entry_<NAME>` ABI the build verifies, so the gate also needs the pin repoint and the local-SDK patch (Review Findings F1).
- **Rationale:** The glibc default build needs no special toolchain, and patching the scaffold to the local SDK makes both gates compile green on a standard runner; they are the regression gate the subcommand lacked.
- **Promotes to ADR:** no

### [4] Scope architecture.md and docs edits to build-model wording only

- **Decision:** In `docs/writing-a-udf.md`, edit only the musl→glibc build-model wording (the "cross-compile to a musl .so" block, the artifact path, the "equivalent to cargo build --target ...musl" line). Edit `specs/architecture.md` L41/L91 directly as a root-level reference doc (same treatment as `mission.md`; it is not delta-merged). `docs/installation.md` needs no edit: on current `main` it carries no musl/static/cdylib/glibc or support-matrix framing; its support-matrix wording is PR #79's to add and keep consistent. Leave arch/Personal-specific notes to PR #79.
- **Alternatives:** Rewrite the full install/writing docs — rejected: PR #79 already adds, and preserves on rebase, a Personal-install note that the UDF `.so` must be built in a Linux environment matching the deployment architecture (its Finding B); overreaching would clobber #79's fix.
- **Rationale:** Keeps the two PRs' doc edits disjoint and each PR's diff reviewable.
- **Promotes to ADR:** no

### [5] PR #79 rebase coordination

- **Decision:** #80 records first and takes decision number `024` (`specs/_decision/024-fix-glibc-cdylib-build-model.md`, existing records run 001-023). On rebase onto post-#80 `main`, PR #79 renumbers its arm64 ADR `024`→`025` and drops the content #80 supersedes: its `build.rs` `<host-arch>-unknown-linux-musl` default and retained `ensure_musl_target`; its `specs/tools/cargo-exaudf/spec.md` musl-target Background/scenario rewrite and its "build honors an explicit target override" musl scenario; its `_decision/024` musl-target-override scenario; and the musl-primary framing in `writing-a-udf.md` / `cargo-ecosystem.md`. #79's `--target <triple>` parser is redundant with the one this plan introduces (resolve to one at rebase). #79's `targets/*.json` + `COPY targets/` removal (P7) overlaps this plan's removal — #79 drops the redundant deletion. #80 leaves `about.toml`'s musl `targets` pin in place; repointing the license-bundle crate graph to gnu+musl for both arches is PR #79's P2 license task, so #80 defers it to avoid colliding with that rewrite (see plan.md § Dead Code Removal). #79 keeps its genuine value: arch-neutralization of the *glibc* path (host arch via `std::env::consts::ARCH`), Exasol Personal install, the license bundle, the CI arm leg, the Dockerfile multiarch derivation, and its Finding B/C fixes.
- **Alternatives:** Amend PR #79 in place from this plan — rejected: #79 owns its own rebase; #79 already declares `Depends on #80`.
- **Rationale:** Records the merge-order contract so neither PR double-removes files nor collides on the ADR number.
- **Promotes to ADR:** no

### [6] Out-of-scope: cargo exaudf vs cargo exasol-udf command name

- **Decision:** Leave the `specs/tools/cargo-exaudf/spec.md` command name as `cargo exaudf` (the file's existing convention) in this plan. The binary, `crates/cargo-exasol-udf/src/main.rs` usage text, and `specs/mission.md` all say `cargo exasol-udf`; the spec disagrees. Record it as a suggested separate follow-up issue rather than widening this plan.
- **Alternatives:** Fix the name mismatch here — rejected: not a musl claim; unrelated to the build-model reconciliation; would broaden the diff.
- **Rationale:** Keeps this plan scoped to issue #80; the naming mismatch is a distinct, non-blocking cleanup.
- **Promotes to ADR:** no

## Review Findings

### [plan-review] Scaffold SDK-version contract blocks the un-ignored build gate

- **Finding:** `build_produces_host_cdylib` — the regression gate the plan un-ignores — cannot pass as written. The `new` scaffold pins `exasol-udf-sdk`/`exasol-udf-macros` at `"0.1"`, which neither resolves nor compiles against the `0.21.x` workspace SDK whose `__exa_udf_entry_<NAME>` named-entry ABI the build's post-verification requires. The plan misattributed the `#[ignore]` solely to musl.
- **Direction change:** Added Group A task 1.3 repointing the scaffold pins to `"0.21"`, and amended task 1.1 so the test injects a `[patch.crates-io]` pointing `exasol-udf-sdk`/`exasol-udf-macros` at the in-repo crate paths — the gate now compiles the scaffold against the local SDK with no crates.io dependency. Amended the `cargo-exaudf` scenario "new scaffolds a buildable UDF crate" to record the contract (pins track the current SDK major.minor; the scaffold MUST build against it). Task 1.3 tagged `[expert]` (FFI/ABI + test-harness sensitive).
- **Promotes to ADR:** no

### [plan-review] Target-override scenario had no executing test

- **Finding:** The NEW `MUST` scenario "build honors an explicit target override" mapped only to `build_honors_target_override`, which stayed `#[ignore]`d with a placeholder `<triple>` — a recorded requirement with no CI pass/fail value.
- **Direction change:** Un-`#[ignore]` the test and run it in CI using the host triple `x86_64-unknown-linux-gnu` as the concrete `--target` (installed by definition), asserting `target/<host-triple>/release/lib<crate>.so`. Updated tasks.md 1.1 and plan.md § Verification.
- **Promotes to ADR:** no

### [plan-review] Unverified "glibc-dynamic" qualifier in the cdylib scenario

- **Finding:** Scenario "build produces a loadable host cdylib" asserted the artifact MUST be "glibc-dynamic", but task 1.1 asserts only the printed path and the exported symbol — the dynamic-link property was never verified.
- **Direction change:** Dropped the "glibc-dynamic"/"dlopens" qualifier from the scenario THEN clause, leaving "a cdylib that exports at least one `__exa_udf_entry_<NAME>` symbol" — the simpler verifiable option, consistent with the test's assertions.
- **Promotes to ADR:** no

### [plan-review] Task 3.4 (installation.md) had no target on main

- **Finding:** Task 3.4 reconciled a musl "support matrix" in `docs/installation.md`, but that framing does not exist on current `main` (it is a PR #79 addition) — a no-op that risked an implementer inventing a section colliding with #79.
- **Direction change:** Deleted task 3.4, renumbered 3.5→3.4, removed `docs/installation.md` from decision-log [4], and recorded that installation.md's build-model wording is unaffected on `main` and its support-matrix wording is #79's. Tasks 3.1-3.3 already cover all genuine README/docs musl references (verified).
- **Promotes to ADR:** no

### [plan-review] about.toml still pins the musl license-bundle target

- **Finding:** `about.toml:24` keeps `targets = ["x86_64-unknown-linux-musl"]`, consumed by the CI license step, so the shipped `THIRD-PARTY-LICENSES.md` under-reports gnu-gated deps once the musl target is dropped.
- **Direction change:** Deferred to PR #79's P2 license task (which rewrites `about.toml` to gnu+musl for both arches), mirroring task 2.4's `targets/*.json` overlap note — #80 must not collide with that rewrite. Recorded the deferral in plan.md § Dead Code Removal and a coordination line in decision-log [5].
- **Promotes to ADR:** no
