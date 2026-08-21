# Code Review Findings: change-slc-runtime-debian

## Summary
- Files reviewed: 30 (23 modified, 1 deleted, 6 new)
- Total findings: 18 (standard: 15, expert: 3)

Verified green before review: `cargo clippy -p cargo-exasol-udf --all-targets --all-features -- -D warnings` (clean), `cargo test -p cargo-exasol-udf` (20 unit + 13 validate + 5 build + 2 new = all pass), `bash dist/tests/os_licenses_test.sh` (pass).

Verified consistent (no findings): the decision-log entry [5] bzip2 correction landed everywhere — no live file claims `exaudfclient` links bzip2, `dist/tests/slc_tarball_test.sh:55-58` actively forbids a `libbz2*`/`libzmq*` `DT_NEEDED` on the client, and neither `apt-get install` in `Dockerfile` names `libbz2-dev`. The glibc floor is `2.41` everywhere it appears with no drift, and `slc-glibc-floor.txt` is `2.41\n` which all three readers trim safely. The Alpine/musl/apk purge is complete across the changed-file set; the only remaining hits are the deliberate musl-absence guards the plan named as keepers. Every test name in the plan's Scenario Coverage table exists in the implementation.

## Standard fixes

### crates/cargo-exasol-udf/src/validate.rs

#### [MIXED_ABSTRACTION_LEVEL] `run` interleaves orchestration, reporting and vtable probing
- Location: lines 52-154
- Issue: `run` now spans ~100 lines across five abstraction levels — argument parsing, a filesystem existence probe, ELF reading, two report-and-decide blocks that both format stdout lines and return errors, and the per-UDF dlopen loop with its own error accumulator. The two new blocks (lines 70-104) each mix a policy decision (does this exceed the floor / is this dependency staged) with output formatting, so neither can be tested or reused without running the whole subcommand.
- Fix: In crates/cargo-exasol-udf/src/validate.rs, extract two private functions from `run` — `report_glibc_floor(artifact: &elf::SharedObject, so_path: &Path) -> Result<(), String>` covering lines 70-83 and `report_dynamic_dependencies(artifact: &elf::SharedObject, so_path: &Path, deny_unknown_deps: bool) -> Result<(), String>` covering lines 85-104 — and extract the dlopen loop at lines 106-147 into `verify_vtables(artifact: &elf::SharedObject, so_path: &Path) -> Result<(), String>`. Leave `run` as guard clauses plus four calls in the plan's documented order (ELF read → entry symbols → glibc floor → DT_NEEDED → dlopen).

#### [DUPLICATE_OPERATION] `so_path.exists()` pre-check duplicates `elf::read`'s unreadable path
- Location: lines 56-58
- Issue: since the `nm` shell-out was replaced, `elf::read` already reports a missing file as `ElfError::Unreadable { path, reason }`, rendered as `reading '<path>': No such file or directory`. The surviving `exists()` pre-check gives the same condition a second, different message (`file not found: '<path>'`) and a second stat syscall. `crates/cargo-exasol-udf/tests/validate.rs:313-323` accepts either wording, so removing the pre-check keeps `validate_rejects_missing_file` green.
- Fix: In crates/cargo-exasol-udf/src/validate.rs, delete the `if !so_path.exists()` block at lines 56-58 so a missing file is reported once by `elf::read`, then run `cargo test -p cargo-exasol-udf --test validate validate_rejects_missing_file` and confirm it still passes.

#### [SWALLOWED_ERROR] An unrecognized flag is silently ignored, making `--deny-unknown-deps` typo-fragile
- Location: lines 159-173, the `_ => {}` arm at line 166
- Issue: `parse_validate_args` accepts the artifact path from the first non-matching argument and then silently discards every later argument. A mistyped strict flag (`validate lib.so --deny-unknown-dep`) is dropped without a word, so `validate` exits 0 and CI silently loses the gate the flag exists to provide — the failure mode is a passing check over unchecked behaviour. A mistyped flag placed first is instead consumed as the artifact path, producing the confusing `file not found: '--deny-unknown-dep'`.
- Fix: In crates/cargo-exasol-udf/src/validate.rs, change `parse_validate_args`' final match arm from `_ => {}` to return `Err(format!("unrecognized argument '{other}'. Usage: cargo exasol-udf validate <path-to-so> [--deny-unknown-deps]"))` for any argument starting with `--`, and the same for a second bare argument once `path` is set. Add a test to crates/cargo-exasol-udf/tests/validate.rs named `validate_rejects_a_mistyped_deny_flag` asserting a non-zero exit and that stderr names the unrecognized argument.

### crates/cargo-exasol-udf/src/main.rs

#### [MISSING_BOUNDARY_TEST] The new `--deny-unknown-deps` usage text has no assertion
- Location: lines 28-52 (`write_usage`)
- Issue: `write_usage` gained five new lines documenting `--deny-unknown-deps`, but `crates/cargo-exasol-udf/src/main_tests.rs` was not extended. Its single test is named `write_usage_lists_every_subcommand_and_the_target_flag` and asserts `text.contains("validate <path>")`, which the new line `  validate <path> [--deny-unknown-deps]` still satisfies — so the test name's promise ("every subcommand and the target flag") no longer covers the flag this change added, and the usage text can regress silently.
- Fix: In crates/cargo-exasol-udf/src/main_tests.rs, add `assert!(text.contains("--deny-unknown-deps"));` and an assertion for the explanatory phrase `"outside that surface"` to `write_usage_lists_every_subcommand_and_the_target_flag`, and rename the test to `write_usage_lists_every_subcommand_and_flag`.

### crates/cargo-exasol-udf/src/slc_surface.rs

#### [OUTDATED_COMMENT] `glibc_floor`'s doc comment states two things the code does not do
- Location: lines 34-43
- Issue: the doc says the floor is "parsed once from the committed `slc-glibc-floor.txt`" — it is re-parsed on every call, and `validate::run` calls it twice per invocation (directly at validate.rs:70 and again inside `check_against_floor`). It also says "this function is the floor's only reader, so no other module can drift from it", but `crates/cargo-exasol-udf/tests/validate.rs:30` is a second independent `include_str!("../slc-glibc-floor.txt")` reader. Both claims are load-bearing design statements, so a reader trusts them.
- Fix: In crates/cargo-exasol-udf/src/slc_surface.rs, wrap the parse in a `std::sync::LazyLock<GlibcVersion>` so `glibc_floor()` genuinely parses once and returns a reference to it, and reword the second paragraph to say the CLI reads the floor only here while the tarball build proves it still matches the shipped `libc.so.6` — dropping the false "only reader" claim.

### crates/cargo-exasol-udf/src/slc_surface_tests.rs

#### [IMPLEMENTATION_COUPLED_TEST] Boundary test hardcodes the floor value instead of reading it
- Location: lines 14-18 (`floor_check_accepts_glibc_at_the_floor`), line 5 (`committed_floor_parses`)
- Issue: `floor_check_accepts_glibc_at_the_floor` builds its "at the floor" input from the literal `"2.41"` rather than from `glibc_floor()`, so the moment the committed floor moves this test stops exercising the boundary it is named for and instead asserts that a now-below-floor version is within the floor — it keeps passing while its stated boundary goes untested. `committed_floor_parses` also hardcodes `"2.41"`, but there the literal is a deliberate canary; its name is the defect, since it claims to test parsing while asserting a specific published value.
- Fix: In crates/cargo-exasol-udf/src/slc_surface_tests.rs, change `floor_check_accepts_glibc_at_the_floor` to call `check_against_floor(&glibc_floor())` so the boundary follows the committed value, and rename `committed_floor_parses` to `committed_floor_is_the_published_container_floor` (keeping its `"2.41"` literal as the intentional drift canary).

### crates/cargo-exasol-udf/src/elf.rs

#### [OUTDATED_COMMENT] `read`'s doc claims no module needs binutils, but the crate's own test suite still shells out to `nm`
- Location: lines 83-87
- Issue: the doc comment promises "no other module needs an ELF reader — or a `binutils` install — of its own", which is the stated rationale for decision-log entry [7] ("two mechanisms reading the same ELF is back-door leakage"). `crates/cargo-exasol-udf/tests/build.rs:98-105` still runs `Command::new("nm").args(["--dynamic", "--defined-only"])` in its `entry_symbols` helper, so binutils remains a hard requirement of this crate's test suite and the ELF symbol-table format is still known in two places.
- Fix: In crates/cargo-exasol-udf/tests/build.rs, replace the `entry_symbols` helper's `nm` shell-out with an assertion driven through the CLI — invoke the built `cargo-exasol-udf` binary with `exasol-udf validate <so>` and parse the reported UDF names from its stdout (an integration test of a bin-only crate cannot import `crate::elf`) — then delete the helper's `nm` code path. Leave the doc comment in crates/cargo-exasol-udf/src/elf.rs unchanged once no `nm` call remains in the crate; if the `nm` call must stay, instead reword lines 86-87 to scope the claim to the CLI's own modules.

### crates/cargo-exasol-udf/tests/validate.rs

#### [DUPLICATE_TEST] Two tests assert the same missing-entry behaviour, one of them via the host filesystem
- Location: lines 35-57 (`system_library_without_entry_symbols`), 326-346 (`validate_rejects_missing_entry_symbol`), 545-572 (`build_verifies_named_entry`)
- Issue: `validate_rejects_missing_entry_symbol` and `build_verifies_named_entry` run the same command against a `.so` with no `__exa_udf_entry_*` export and make the identical four-way `contains` assertion on stderr. The former additionally depends on a host `libm.so.6` at one of three guessed multiarch paths; the new `system_library_without_entry_symbols` helper turns a missing one into a hard `panic!`, so the suite now fails outright on any host whose libm is not at those paths (a non-multiarch or non-Debian-layout host) rather than on a defect in the code under test. `build_verifies_named_entry` already covers the same behaviour deterministically with a generated cdylib.
- Fix: In crates/cargo-exasol-udf/tests/validate.rs, delete `validate_rejects_missing_entry_symbol` and the `system_library_without_entry_symbols` helper, and rename `build_verifies_named_entry` to `validate_rejects_a_so_without_any_named_entry_symbol`, keeping its generated-fixture body and its doc comment about `build::run`'s reliance on the same predicate. Then run `cargo test -p cargo-exasol-udf --test validate` and confirm 12 tests pass.

#### [STANDARD_LIBRARY_DUPLICATE] Hand-rolled `TempDir` next to a `tempfile` dev-dependency the sibling new test file uses
- Location: lines 270-300
- Issue: the comment "Minimal tempdir without extra dependency" is false — `tempfile = "=3.14.0"` is already a `[dev-dependencies]` entry of this crate, and the new `crates/cargo-exasol-udf/src/elf_tests.rs:122` uses `tempfile::tempdir()`. The hand-rolled version reimplements it with a PID+nanos name, no collision retry, and a `Drop` that discards its error, leaving two temp-directory idioms in one crate after this change.
- Fix: In crates/cargo-exasol-udf/tests/validate.rs, delete the `tempdir()` function, the `TempDir` struct and its `path`/`Drop` impls, replace every `tempdir()` call with `tempfile::tempdir().expect("create tempdir")`, and change the `dir.path()` call sites to the `tempfile::TempDir::path` equivalent.

#### [TOO_MANY_ARGUMENTS] Three new fixture-compilation helpers take four arguments each
- Location: lines 67-72 (`compile_fixture_linked_against`), 85 (`compile_cdylib`), 105-110 (`compile_shared_stub`)
- Issue: each helper takes four positional arguments of which three are `&str`/`&Path`, so a transposed pair at a call site compiles cleanly and produces a differently-named fixture — the guardrail limit exists precisely for this shape, and it applies to test code the same as production code.
- Fix: In crates/cargo-exasol-udf/tests/validate.rs, introduce `struct CdylibFixture<'a> { out_dir: &'a Path, name: &'a str, source: &'a str }` and `struct SharedStub<'a> { out_dir: &'a Path, file_name: &'a str, symbol: &'a str }`, change `compile_cdylib` to `fn compile_cdylib(fixture: CdylibFixture<'_>, extra_args: &[String])`, `compile_fixture_linked_against` to `fn compile_fixture_linked_against(fixture: CdylibFixture<'_>, link_name: &str)` and `compile_shared_stub` to `fn compile_shared_stub(stub: SharedStub<'_>, link_args: &[String])`, and update every call site.

### dist/tests/os_licenses_test.sh

#### [ASSERTION_FREE_TEST] The bzip2-license assertion is satisfied by the template's own prose
- Location: lines 122-125
- Issue: the check is `grep -qi "bzip2" "$OS_MANIFEST"` with the failure message "missing bzip2 license text", but `dist/os-licenses.hbs` writes the word `bzip2` into the header prose (line 9), the staged-library table (line 33) and the source offer (line 46). The assertion therefore passes even if `cargo about` emits no `bzip2-1.0.6` text at all — which is exactly the condition plan task 4.2 said to detect and compensate for. Verified against the current `dist/THIRD-PARTY-OS-LICENSES.md`: the real text renders under the heading `## bzip2 and libbzip2 License v1.0.6 (bzip2-1.0.6)` at line 994, so a distinctive anchor is available.
- Fix: In dist/tests/os_licenses_test.sh, replace the `grep -qi "bzip2"` check with `grep -qF "bzip2 and libbzip2 License v1.0.6 (bzip2-1.0.6)"` and add a second `grep -qF "Julian R Seward"` assertion, so the test fails if only the table mention survives.

#### [NONDETERMINISTIC_TEST] The manifest is generated only when absent, so the test subject depends on prior filesystem state
- Location: lines 76-89
- Issue: `dist/THIRD-PARTY-OS-LICENSES.md` is git-ignored. When it is absent the test generates it (a side effect that writes into the repo from inside a test); when it is present the test asserts over whatever a previous run left behind, however stale. The same command therefore validates freshly generated output on a clean checkout and a possibly outdated file on a developer machine — and CI already runs `dist/generate-licenses.sh` immediately before this test (`.github/workflows/ci.yml:346-349`), so the generation branch only ever fires locally.
- Fix: In dist/tests/os_licenses_test.sh, remove the conditional `bash "$GENERATE_SCRIPT"` invocation from `os_manifest_covers_staged_library_set` and replace it with a `fail` when `$OS_MANIFEST` does not exist, whose message instructs the caller to run `bash dist/generate-licenses.sh` first.

### dist/tests/slc_tarball_test.sh

#### [OUTDATED_COMMENT] `resolve_soname`'s comment claims a library-directory lookup the index does not perform
- Location: lines 122-134 with `index_tree` at lines 136-141
- Issue: the comment states "The loader finds a soname by looking the name itself up in the library directories, so a soname resolves only through a tree entry of that name". `TREE_PATH_BY_BASENAME` is populated from `find "$TREE" \( -type f -o -type l \)` — every file and link anywhere in the tree — so a library staged outside the directories named in `etc/ld.so.conf.d/<triplet>.conf` still resolves and `slc_tarball_library_surface_present` / `slc_tarball_dt_needed_closure_is_complete` pass on a tree the loader could not actually resolve. The map is also keyed on basename alone, so two tree entries sharing a basename silently collapse to whichever `find` emitted last.
- Fix: In dist/tests/slc_tarball_test.sh, restrict `index_tree`'s `TREE_PATH_BY_BASENAME` population to the directories listed in `$TREE/etc/ld.so.conf.d/*.conf` plus `$(dirname "$TREE$LOADER_PATH")`, and `fail` from `index_tree` when two entries within those directories share a basename. Keep the separate `find "$TREE" -type f` walk that builds `STAGED_ELF_FILES` unrestricted.

#### [MAGIC_NUMBER] Symlink-hop limit is a bare literal
- Location: line 107
- Issue: `for ((hop = 0; hop < 16; hop++))` bounds the symlink-chase loop on an unexplained 16; the value is the loop's loop-detection threshold and appears nowhere else, so a reader cannot tell whether it mirrors a loader constant or is arbitrary.
- Fix: In dist/tests/slc_tarball_test.sh, add a top-level constant `MAX_SYMLINK_HOPS=16` beside the other configuration constants and use it as the loop bound in `resolve_in_tree`.

### Cargo.toml

#### [DEAD_FLEXIBILITY] `goblin` is added with default features, compiling five unused binary-format parsers
- Location: line 105 (`goblin = "0.9"` in `[workspace.dependencies]`)
- Issue: verified in `goblin-0.9.3`'s own `Cargo.toml`, `default = ["std", "elf32", "elf64", "mach32", "mach64", "pe32", "pe64", "te", "archive", "endian_fd"]`. `crates/cargo-exasol-udf/src/elf.rs` uses only `goblin::elf`, so the Mach-O, PE, TE and archive parsers are compiled and never reached, and the `mach*`/`archive` features are what drag `plain 0.2.3` into `Cargo.lock` (visible in this change's lockfile diff).
- Fix: In Cargo.toml, change the `[workspace.dependencies]` entry to `goblin = { version = "0.9", default-features = false, features = ["std", "elf32", "elf64", "endian_fd"] }`, run `cargo update -p goblin` (or `cargo check -p cargo-exasol-udf`) to regenerate Cargo.lock, confirm `plain` no longer appears in Cargo.lock, and confirm `cargo test -p cargo-exasol-udf` still passes.

## Expert fixes

### crates/cargo-exasol-udf/src/slc_surface.rs

#### [INFORMATION_LEAKAGE] The SLC library surface is declared independently in three enforcement points
- Location: lines 5-21 (`ALLOWED_SONAMES`), with `Dockerfile:93-97` and `dist/tests/slc_tarball_test.sh:27-43`
- Issue: the plan's Design section justifies this module's existence precisely as the owner of "*what the SLC provides*", so the decision "would otherwise leak into `validate.rs`, the tarball test and the docs independently". The implementation did not achieve that: the same 15-soname list is written out three times — the `for lib in ...` staging loop in `Dockerfile`, `ALLOWED_SONAMES` here, and `SLC_LIBRARY_SURFACE` in `dist/tests/slc_tarball_test.sh` — with nothing enforcing agreement between them. The three copies happen to match today (verified soname by soname), which is exactly why the divergence would be silent: staging a new library without updating `ALLOWED_SONAMES` makes `validate` warn about a library the container *does* ship, and adding it to `ALLOWED_SONAMES` without staging it makes `validate` bless an artifact that cannot load — neither shows up as a test failure. The same change proves the fix is available in this repository: the glibc floor was given exactly this treatment, one committed file read by both the CLI (`include_str!`) and the shell test.
- Fix: Add `crates/cargo-exasol-udf/slc-library-surface.txt` containing one soname per line (the 15 entries currently in `ALLOWED_SONAMES`), parse-simple like `slc-glibc-floor.txt` so a shell loop can read it. In crates/cargo-exasol-udf/src/slc_surface.rs replace the `ALLOWED_SONAMES` literal array with a `std::sync::LazyLock<Vec<&'static str>>` built from `include_str!("../slc-library-surface.txt")` (trim, skip blank lines) and document the file as the single owner. In `Dockerfile`, have the builder stage copy that file into `/slc-meta/library-surface` alongside `triplet` and `loader`, and drive the staging `for lib in ...` loop from `$(cat /slc-meta/library-surface)` instead of the inline list. In dist/tests/slc_tarball_test.sh, replace the `SLC_LIBRARY_SURFACE` array literal with a read of `$ROOT/crates/cargo-exasol-udf/slc-library-surface.txt` (mirroring how `GLIBC_FLOOR_FILE` is already read at line 18). Add a unit test in crates/cargo-exasol-udf/src/slc_surface_tests.rs asserting the parsed surface is non-empty and contains `libc.so.6`, then rebuild the tarball and run `bash dist/tests/slc_tarball_test.sh` to confirm the surface assertions still pass.

### crates/cargo-exasol-udf/src/validate.rs

#### [SWALLOWED_ERROR] `enumerate_entry_symbols` changed from "empty on unreadable" to "error", and one caller still discards it
- Location: lines 181-185
- Issue: the plan's task 5.4 kept the signature so `build.rs`'s artifact check "keeps working", but the semantics changed underneath it. The removed `nm` path returned `Ok(Vec::new())` for anything it could not parse; `elf::read` now returns `Err`. `crates/cargo-exasol-udf/src/build.rs:168` still calls `enumerate_entry_symbols(so_path).unwrap_or_default()`, so a genuine read or parse failure is discarded and re-reported as the unrelated `"no __exa_udf_entry_<NAME> symbol found"` from the `ok_or_else` two lines later — the caller is told the artifact exports nothing when the real fact is that it could not be read. It is currently masked only because `build::run` already read the same path successfully at build.rs:57, which also means the same file is now parsed twice per `build` invocation, against decision-log entry [7]'s "one ELF read" rationale.
- Fix: In crates/cargo-exasol-udf/src/build.rs, change `maybe_emit_sidecar` to accept the already-derived entry names — add a `entry_names: &[String]` parameter and pass the `entry_names` binding from build.rs:57 at the call site — so the second `enumerate_entry_symbols` call and its `unwrap_or_default()` are both deleted and the ELF is read once per build. Keep `enumerate_entry_symbols` in crates/cargo-exasol-udf/src/validate.rs for the remaining build.rs:57 caller. Then run `cargo test -p cargo-exasol-udf --test build` and confirm all five tests still pass.

### Dockerfile

#### [MISSING_BOUNDARY_TEST] The `/slc/tmp` build-trace hotfix is untested and hardcodes a single client-internal filename
- Location: lines 158-160, with `dist/tests/slc_tarball_test.sh:66-70` and its runner list at lines 593-622
- Issue: the correction recorded as tasks.md 2.6 removes exactly `/slc/tmp/exaudf_started.txt` after the chroot self-test, but no assertion anywhere prevents the leak from returning — `FORBIDDEN_PAYLOAD_PATHS` does not cover `tmp/`, and the size-ceiling check cannot see a file this small. It is also narrower than the condition it fixes: the Dockerfile's own comment at lines 82-84 states the client writes "its startup **and connect-back** traces" into `/tmp`, so any second trace file the self-test produces still ships. Nothing verifies the `chmod 1777` on `/slc/tmp` survives `tar --hard-dereference` either, and a non-writable `/tmp` inside the sandbox silently degrades UDF diagnostics.
- Fix: In Dockerfile, replace `RUN rm -f /slc/tmp/exaudf_started.txt` with `RUN find /slc/tmp -mindepth 1 -delete` so every artefact of the self-test is removed regardless of filename, keeping the existing explanatory comment. In dist/tests/slc_tarball_test.sh, add an assertion function `slc_tarball_tmp_is_empty_and_world_writable` that fails when `find "$TREE/tmp" -mindepth 1` returns anything (naming the leaked entries) and when the extracted `tmp/` directory's mode is not `1777`, and register it in the runner list between `slc_tarball_conf_resolver_symlinks` and `slc_tarball_zoneinfo_is_regular_file`. Rebuild the tarball with `docker build --target artifact --output type=local,dest=/tmp/slc .` and run `bash dist/tests/slc_tarball_test.sh /tmp/slc/lc-rs.tar.gz` to confirm the new assertion passes.
