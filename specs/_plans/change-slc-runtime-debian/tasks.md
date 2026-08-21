# Tasks: change-slc-runtime-debian

## Phase 0: Version bump (done first, per operator instruction)
- [x] 0.1 Bump `[workspace.package].version` to 0.23.0 and the `exasol-udf-sdk` dependency entry; regenerate `Cargo.lock`.

## Phase 1: Shared contract
- [x] 1.1 Add `crates/cargo-exasol-udf/slc-glibc-floor.txt` containing exactly `2.41`, documented by a doc comment at its only Rust reader.

## Phase 2: Container build (Group A — sequential, one file)
- [x] 2.1 Add root `Dockerfile` builder stage on `rust:1.94-trixie`. [expert]
- [x] 2.2 Add the `debian:trixie-slim` staging stage. [expert]
- [x] 2.3 Stage the payload into `/slc`, chroot self-test, tar, `FROM scratch AS artifact` stage.
- [x] 2.4 Delete `Dockerfile.alpine`.
- [x] 2.5 CORRECTION: remove `libbz2-dev` from the builder's `apt-get install` list — `exaudfclient` links no bzip2 at all (verified across 3 real builds; root cause: exarrow-rs's bzip2 usage is confined to its CSV IMPORT/EXPORT local-file-compression path, unreachable from ExaConnection). Plan, decision-log entry 5, and container/slim-image spec.md updated to match. `libbz2.so.1` staging in `/slc` is unaffected (kept for UDF authors' own crates).
- [x] 2.6 CORRECTION: remove the leftover `/slc/tmp/exaudf_started.txt` build-time debug trace before `tar`, so it doesn't leak into every shipped artifact (flagged by task 3.1's tarball-test author).

## Phase 3: Tarball contract test and pipeline wiring (Group D)
- [x] 3.1 Add `dist/tests/slc_tarball_test.sh <tarball>`. [expert]
- [x] 3.2 Wire the new test and Dockerfile through `ci.yml`, `scripts/ci-it-local.sh`, `scripts/install.sh`, `benches/README.md`, `crates/it/src/lib.rs`.

## Phase 4: OS license bundle (Group B — independent, may run concurrently)
- [x] 4.1 Rewrite `dist/os-attribution/Cargo.toml` license expression and `dist/about-os.toml` accepted list.
- [x] 4.2 Rewrite `dist/os-licenses.hbs`; update `dist/generate-licenses.sh` comments; append bzip2 license text if needed.
- [x] 4.3 Add `dist/tests/os_licenses_test.sh`; wire into `build-slc` after `dist/generate-licenses.sh`.

## Phase 5: cargo exasol-udf validate platform checks (Group C — sequential)
- [x] 5.1 Add `goblin` to `[workspace.dependencies]` and `crates/cargo-exasol-udf/Cargo.toml`; regenerate `Cargo.lock`.
- [x] 5.2 Add `crates/cargo-exasol-udf/src/elf.rs` (+ `elf_tests.rs`). [expert]
- [x] 5.3 Add `crates/cargo-exasol-udf/src/slc_surface.rs` (+ `slc_surface_tests.rs`).
- [x] 5.4 Replace `validate::enumerate_entry_symbols`'s `nm` shell-out with `elf::read`; drop dead binutils error path. [expert]
- [x] 5.5 Add platform checks to `validate::run`; parse `--deny-unknown-deps`; extend `write_usage` in `main.rs`.
- [x] 5.6 Extend `crates/cargo-exasol-udf/tests/validate.rs` with new fixtures/cases. [expert]

## Phase 6: Purge Alpine/musl vocabulary (Group E)
- [x] 6.1 `specs/mission.md`, `specs/architecture.md` — fix Tech Stack row and collapse Dockerfile tree entries.
- [x] 6.2 `docs/writing-a-udf.md`, `docs/installation.md` — builder image, glibc floor, SLC surface, new validate checks.
- [x] 6.3 `CLAUDE.md` — remove moot Alpine-vs-Debian bullet, record kept facts.
- [x] 6.4 `crates/it/tests/db_roundtrip.rs` — reword tzdata scenario comment.
- [x] 6.5 ADRs — reword ADR 025 usr-merge parenthetical; add pointer line to ADR 024.

## Phase 7: Verification
- [x] 7.1 Run build/test/lint/format/license checklist per plan `## Verification > Checklist`. All green: build, 338 unit tests, clippy, fmt, cargo deny, and the live-DB integration suite (`db_roundtrip_all_scenarios`, 36 sub-scenarios, dedicated `exasol/docker-db:2026.1.0` container).
- [x] 7.2 Scenario coverage audit against plan `## Verification > Scenario Coverage`. Every listed test exists and passes.
- [x] 7.3 Manual verification steps per plan `## Verification > Manual Testing`. All done, including the live-DB integration run.
- [x] 7.4 Generate verification-report.md.

## Phase 4: Review Fixes (Expert)
- [x] 4.1 Extract the 15-soname SLC library surface into `crates/cargo-exasol-udf/slc-library-surface.txt` and drive `slc_surface.rs`, the `Dockerfile` staging loop and `dist/tests/slc_tarball_test.sh` from it. [expert]
- [x] 4.2 Thread the already-derived entry names into `build::maybe_emit_sidecar` so the ELF is read once per build and no read error is discarded. [expert]
- [x] 4.3 Broaden the Dockerfile `/slc/tmp` cleanup to `find /slc/tmp -mindepth 1 -delete` and add the `slc_tarball_tmp_is_empty_and_world_writable` tarball assertion. [expert]

## Phase 4: Review Fixes (Standard, batch 3)
- [x] 4.1 [DUPLICATE_TEST] In crates/cargo-exasol-udf/tests/validate.rs, delete `validate_rejects_missing_entry_symbol` and the `system_library_without_entry_symbols` helper, and rename `build_verifies_named_entry` to `validate_rejects_a_so_without_any_named_entry_symbol`, keeping its generated-fixture body and its doc comment about `build::run`'s reliance on the same predicate. Run `cargo test -p cargo-exasol-udf --test validate` and confirm the expected test count passes.
- [x] 4.2 [STANDARD_LIBRARY_DUPLICATE] In crates/cargo-exasol-udf/tests/validate.rs, delete the `tempdir()` function, the `TempDir` struct and its `path`/`Drop` impls, replace every `tempdir()` call with `tempfile::tempdir().expect("create tempdir")`, and change the `dir.path()` call sites to the `tempfile::TempDir::path` equivalent.
- [x] 4.3 [TOO_MANY_ARGUMENTS] In crates/cargo-exasol-udf/tests/validate.rs, introduce `struct CdylibFixture<'a> { out_dir: &'a Path, name: &'a str, source: &'a str }` and `struct SharedStub<'a> { out_dir: &'a Path, file_name: &'a str, symbol: &'a str }`, change `compile_cdylib` to `fn compile_cdylib(fixture: CdylibFixture<'_>, extra_args: &[String])`, `compile_fixture_linked_against` to `fn compile_fixture_linked_against(fixture: CdylibFixture<'_>, link_name: &str)` and `compile_shared_stub` to `fn compile_shared_stub(stub: SharedStub<'_>, link_args: &[String])`, and update every call site.

## Phase 4: Review Fixes (Standard, batch 1)
- [x] 4.1 [MIXED_ABSTRACTION_LEVEL] Extract `report_glibc_floor`, `report_dynamic_dependencies`, and `verify_vtables` private functions out of `validate::run` in crates/cargo-exasol-udf/src/validate.rs.
- [x] 4.2 [DUPLICATE_OPERATION] Delete the `so_path.exists()` pre-check in `validate::run` (crates/cargo-exasol-udf/src/validate.rs) so a missing file is reported once by `elf::read`.
- [x] 4.3 [SWALLOWED_ERROR] Make `parse_validate_args` in crates/cargo-exasol-udf/src/validate.rs return an error naming an unrecognized argument instead of silently discarding it; add `validate_rejects_a_mistyped_deny_flag` to crates/cargo-exasol-udf/tests/validate.rs.
- [x] 4.4 [MISSING_BOUNDARY_TEST] Extend and rename `write_usage_lists_every_subcommand_and_the_target_flag` to `write_usage_lists_every_subcommand_and_flag` in crates/cargo-exasol-udf/src/main_tests.rs, asserting `--deny-unknown-deps` and `"outside that surface"`.

## Phase 4: Review Fixes (Standard, batch 2)
- [x] 4.4 [OUTDATED_COMMENT] `slc_surface.rs`: wrap `glibc_floor()`'s parse in a `std::sync::LazyLock<GlibcVersion>` so it genuinely parses once, and reword its doc comment to drop the false "only reader" claim.
- [x] 4.5 [IMPLEMENTATION_COUPLED_TEST] `slc_surface_tests.rs`: change `floor_check_accepts_glibc_at_the_floor` to call `check_against_floor(&glibc_floor())` instead of hardcoding `"2.41"`, and rename `committed_floor_parses` to `committed_floor_is_the_published_container_floor`.
- [x] 4.6 [OUTDATED_COMMENT] `elf.rs`/`tests/build.rs`: replace the `entry_symbols` helper's `nm` shell-out in `crates/cargo-exasol-udf/tests/build.rs` with a CLI-driven assertion via `exasol-udf validate <so>`, parsing reported UDF names from stdout.

## Phase 4: Review Fixes (Standard, batch 4)
- [x] 4.1 [ASSERTION_FREE_TEST] In dist/tests/os_licenses_test.sh, replace the `grep -qi "bzip2"` check with a `grep -qF "bzip2 and libbzip2 License v1.0.6 (bzip2-1.0.6)"` assertion plus a `Julian R Seward` assertion.
- [x] 4.2 [NONDETERMINISTIC_TEST] In dist/tests/os_licenses_test.sh, remove the conditional `bash "$GENERATE_SCRIPT"` generation from `os_manifest_covers_staged_library_set` and `fail` with an instructive message when the manifest is missing.
- [x] 4.3 [OUTDATED_COMMENT] In dist/tests/slc_tarball_test.sh, restrict `index_tree`'s `TREE_PATH_BY_BASENAME` population to `$TREE/etc/ld.so.conf.d/*.conf` directories plus `$(dirname "$TREE$LOADER_PATH")`, failing on a basename collision within those directories.
- [x] 4.4 [MAGIC_NUMBER] In dist/tests/slc_tarball_test.sh, extract the symlink-hop loop bound into a named `MAX_SYMLINK_HOPS=16` constant.
- [x] 4.5 [DEAD_FLEXIBILITY] In Cargo.toml, narrow `goblin` to `default-features = false, features = ["std", "elf32", "elf64", "endian_fd"]`, regenerate `Cargo.lock`, and confirm `plain` no longer appears in it.

## Sequential dependencies (from plan)
- 1.1 → Group A (2.x), Group C (5.x), Group D (3.x)
- Group A → Group D
- Group A, Group B, Group C → Group E (6.x)
- Group D, Group E → done (version bump already applied at Phase 0, out of order per operator instruction)
