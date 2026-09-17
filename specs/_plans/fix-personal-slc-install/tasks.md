# Tasks: fix-personal-slc-install

## PR Lifecycle
- [x] resolved
- [x] implemented
- [x] version-bumped
- [ ] tested-green
- [ ] recorded
- [ ] pr-ready

## Phase 2: Implementation (Group A: SLC sandbox skeleton)
- [x] 1.1 Add `dist/slc-sandbox-skeleton.txt` listing the 21 mount-point directories, one bare name per line, no comments
- [x] 1.2 Add failing test `slc_tarball_ships_sandbox_skeleton` to `dist/tests/slc_tarball_test.sh`, register it in the runner list
- [x] 1.3 Dockerfile staging stage: `COPY dist/slc-sandbox-skeleton.txt` + `RUN` creating every listed directory under `/slc`, placed after the existing `mkdir -p` and before `RUN tar`
- [x] 1.4 Build the tarball, run `dist/tests/slc_tarball_test.sh` against it, confirm `slc_tarball_staged_surface_within_ceiling` still passes, record measured size
- [x] 1.5 Bump `[workspace.package].version` and the pinned `exasol-udf-sdk` entry to `0.28.1`, regenerate `Cargo.lock`

## Phase 2: Implementation (Group B: Personal local install mechanism)
- [x] 2.1 Add failing tests `rejects_path_unsafe_name_components` and `resolves_shared_bucketfs_dir_from_mapping` to `scripts/tests/install-personal-test.sh`
- [x] 2.2 Add `require_path_segment <flag> <value>`; wire into `--bfs-service`, `--bucket`, `--slc-name` validation in `scripts/install.sh`
- [x] 2.3 Add `deployment_bucketfs_dir <dir> <service> <bucket>` with the three exit-status branches [expert]
- [x] 2.4 Add failing test `extracts_slc_into_shared_bucketfs`
- [x] 2.5 Add `extract_slc_into_shared_bucketfs <dir> <tarball>` [expert]
- [x] 3.1 Add failing test `selects_local_mechanism_from_deployment_directory`
- [x] 3.2 Add `deployment_supports_ssh_transport <dir>`
- [x] 3.3 Add `personal_local_mechanism <dir> <service> <bucket>`
- [x] 3.4 Add failing test `local_mechanisms_share_the_registration_inputs`
- [x] 3.5 Rewire the `"local"` branch of `main` to dispatch on `personal_local_mechanism`
- [x] 3.6 Update `usage()` text and `--deployment` help for both mechanisms
- [x] 4.1 Update `docs/installation.md` Exasol Personal section for both local mechanisms

## Phase 5: Verification
- [x] 5.1 `cargo build --release` — exit 0
- [x] 5.2 `cargo test` — 0 failures in default-members; 3 pre-existing macOS-only `elf::` linker failures in `cargo-exasol-udf` (confirmed identical on unmodified `origin/main`; needs a Linux runner)
- [x] 5.3 `bash scripts/tests/install-personal-test.sh` — All assertions passed
- [x] 5.4 `bash dist/tests/slc_tarball_test.sh` — All tests passed (run inside a Linux arm64 container against the real built artifact; the script itself needs Linux-only tooling absent on macOS)
- [x] 5.5 `cargo fmt --check && cargo clippy --all-targets --all-features -- -D warnings` — 0 errors/warnings
- [x] 5.6 `git diff --name-only | grep Cargo.lock` — Cargo.lock present in the change
- [x] 5.7 Manual: verified against the live macOS arm64 exasol-personal v2.3.0-rc3 `default` deployment — both `scripts/install.sh --deployment default` (shared-directory mechanism) and `exasol slc custom update --alias RUST` returned a Rust scalar UDF result of `ok` instead of the issue's `VM error: Internal error: VM crashed (SQL state: 22002)`

## Phase 4: Review Fixes
- [x] 4.2 Dockerfile: fix swallowed mkdir error + trailing-blank-line loop-status bug in the sandbox-skeleton RUN
- [x] 4.3 dist/tests/slc_tarball_test.sh: fix vacuous pass in slc_tarball_ships_sandbox_skeleton on empty skeleton file
- [x] 4.4 scripts/install.sh: fix outdated comment on deployment_bucketfs_dir failure-status constants
- [x] 4.5 scripts/install.sh: document personal_local_mechanism's errexit-suppression contract
- [x] 4.6 scripts/tests/install-personal-test.sh: fix tautological self-comparison in local_mechanisms_share_the_registration_inputs
- [x] 4.7 docs/installation.md: fix hard-coded bucket path in manual .so copy snippet
- [x] 4.8 [expert] scripts/install.sh: fix EXISTING= read swallowing exapump/parse failures, which can register RUST as the only script language
