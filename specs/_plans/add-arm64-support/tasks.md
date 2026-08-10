# Tasks: add-arm64-support

> Single-PR bundle of all phases. The plan's per-phase version-bump tasks
> (0.2, 1.6, 2.3, 3.4, 4.4, 5.6, 6.4, 7.2) are consolidated into ONE workspace
> bump handled by the `speq:implement-pr` orchestrator after implementation
> (0.21.3 → 0.22.0, `feat`). They are omitted here deliberately.

## Phase 0: Foundation (Group Foundation)
- [x] 0.1 Apply PR #51's arch-neutral four-file change (source: local `pr-51` ref) — arch-derived Dockerfile.alpine TRIPLET/LOADER, arch-neutral rust-toolchain.toml, aarch64 musl linker in .cargo/config.toml, add targets/aarch64-unknown-linux-musl-dylib.json [expert]
- [x] 0.3 Dockerfile.alpine fail-fast guards for empty TRIPLET/LOADER + verify loader staged at exact PT_INTERP path (Alpine /lib real dir) [expert]

## Phase 2: Implementation — Wave 1 (parallel, disjoint file sets)

### Group P1 — cargo-exasol-udf host-arch target + --target (standard)
- [x] 1.1 host_triple() helper in build.rs from std::env::consts::ARCH; build_tests.rs sibling wired via mod tests; unit tests for x86_64/aarch64 mappings
- [x] 1.2 Parse optional --target <triple> in build::run; default host_triple(); remove MUSL_TARGET constant
- [x] 1.3 Thread selected triple through cargo build --target, printed .so path, ensure_musl_target()
- [x] 1.4 tests/build.rs: assert host triple not hardcoded x86_64; add --target override integration test
- [x] 1.5 usage() in main.rs documents --target; update x86_64 refs in README.md (61,68), docs/writing-a-udf.md (11,602,614), docs/cargo-ecosystem.md (84,87)

### Group P2P7 — license bundle + vestigial targets removal (standard)
- [x] 2.1 about.toml targets = {x86_64,aarch64}-unknown-linux-{gnu,musl}; comment recording glibc exaudfclient + cargo-about union over-attribution rationale
- [x] 2.2 Test about_toml_lists_gnu_triples + about_toml_comments_glibc_rationale; regenerate THIRD-PARTY-LICENSES.md via dist/generate-licenses.sh; confirm union adds aarch64/gnu deps without dropping x86_64 set
- [x] 7.1 Remove targets/x86_64-*.json + targets/aarch64-*.json + Dockerfile.alpine `COPY targets/` line; confirm no build regresses

### Group P4 — CI arm64 build+unit-test leg (standard)
- [x] 4.1 Add ubuntu-24.04-arm leg running cargo build --workspace + unit tests only (no coverage/Sonar/IT)
- [x] 4.2 ci.yml comment recording why IT stays x86_64 (docker-db amd64-only; QEMU non-starter)
- [x] 4.3 Attach P3/P4 to current build-slc/unit-test job graph; do NOT block on #67-#70 flattened graph

## Phase 2: Implementation — Wave 2 (after P1: shares README.md)

### Group P5 — Exasol Personal install path (expert)
- [x] 5.1 Extract SCRIPT_LANGUAGES fragment/registration-string assembly (RUST=…#buckets/…/exaudfclient, no leading slash) from install.sh:123 into scripts/lib/script_languages.sh; refactor install.sh to source it; preserve single-value overwrite behavior [expert]
- [x] 5.2 install-personal.sh connection reading + registration: read connection.sshPort + key path from deployment.json every run (never cache); assemble via shared helper; ALTER SYSTEM SET SCRIPT_LANGUAGES over 8563; NEW existing-entry preservation (read current value + append) [expert]
- [x] 5.3 install-personal.sh transport + filesystem-reconciliation half: build aarch64 SLC or accept SLC_TARBALL; scp over SSH port; extract into /var/lib/exa/bucketfs/<service>/<bucket>/<slc>/; confirm reconciliation (manual/live) [expert]
- [x] 5.4 scripts/tests/install-personal-test.sh sourced-function assertions (arch-independent): executable-path fragment format + existing-entry preservation, pointed at scripts/lib/script_languages.sh; wire into unit-test CI leg
- [x] 5.5 README.md (Personal listed first) + docs/installation.md add Personal path

## Phase 2: Implementation — Wave 3 (Group B, after P2: shares ci.yml + installation.md)

### Group P3 — per-arch release SLC tarball asset (standard)
- [x] 3.1 build-slc job → matrix over runner arch (ubuntu-latest + ubuntu-24.04-arm); per-arch distinctly-named artifacts replacing fixed-name lc-tarball; UPDATE integration job download (~354) to fetch x86_64-named artifact so x86_64 IT keeps passing
- [x] 3.2 release job collects both arch tarballs; x86_64 keeps unsuffixed lc-rust-<VER>.tar.gz; add lc-rust-<VER>-aarch64.tar.gz
- [x] 3.3 docs/installation.md Step 1 (~39-47) list both assets; state unsuffixed = x86_64 build

## Phase 2: Implementation — Wave 4 (Group C, after P1+P3+P5)

### Group P6 — docs (standard)
- [x] 6.1 docs/installation.md 22002 troubleshooting note (fragment points at exaudfclient executable, no leading slash)
- [x] 6.2 docs/writing-a-udf.md glibc-cdylib escape hatch (plain cargo build --release -p my-udf; arm64 workaround until P1)
- [x] 6.3 Platform/arch support matrix (Docker-db x86_64 / SaaS / Personal aarch64 → tarball, install path, UDF build target)

## Phase 4: Review Fixes
_(appended by review-fix agents after Phase 4 code review)_
- [x] 4.1 [MISSING_BOUNDARY_TEST] Add fast unit tests for `parse_build_args` in crates/cargo-exasol-udf/src/build_tests.rs covering: empty args → path "." + host default target; single positional arg → path set, target stays host default; `--target aarch64-unknown-linux-musl` → target set; positional path + `--target` → both set; dangling `--target` with no following value → pin actual behavior (fix to return Err if the silent host-default fallback is unintended)
- [x] 4.2 [SKIPPED_TEST] Wire `dist/tests/about_toml_test.sh` into `.github/workflows/ci.yml`'s x86_64 unit-test job (~line 159), alongside the existing `scripts/tests/install-personal-test.sh` step
- [x] 4.3 [INFORMATION_LEAKAGE] Add a cross-reference line to the `script_languages_entry` header comment in scripts/lib/script_languages.sh noting that crates/it/src/lib.rs `SlcRef::script_languages` independently rebuilds the same registration-string format and must be kept in sync

## Phase 3: Verification
- [x] V.1 cargo build --release → exit 0
- [x] V.2 cargo test → 306 passed, 0 failed, 5 ignored
- [x] V.3 cargo clippy --all-targets --all-features -D warnings → 0
- [x] V.4 cargo fmt --check → no changes
- [x] V.5 Scenario coverage audit + verification report
