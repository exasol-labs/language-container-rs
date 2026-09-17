# Plan: add-cross-distro-compatibility

> **Status:** blocked — see open-questions.md

## Summary

Make a Rust UDF buildable on every Linux distribution Exasol's own requirements name, and make the SLC runnable on a deployment that mounts it read-only. Three committed records carry the contract, one CLI flag builds against it, and a distribution matrix verifies the claim.

## Design

### Context

"The Rust SLC must be compatible with all Linux supported by Exasol" fails today in three independent places. Each has its own mechanism.

**Author host.** `cargo exasol-udf build` runs a host `cargo build --release`. That binds the artifact to two host properties. The linker stamps glibc symbol versions from the host glibc, and the SDK bakes the host's `rustc --version` string into the vtable fingerprint. The container accepts an artifact only when the first is at or below its floor of `2.41` and the second equals the rustc that compiled the shipped `exaudfclient`. A distribution whose glibc is above the floor has no host build path at all. A distribution-packaged Rust toolchain never matches the fingerprint, whatever its version number. `validate` catches neither case for the rustc identity, because it compares an artifact against the CLI's own fingerprint, and the CLI was compiled by the same host rustc that built the artifact.

**SLC root filesystem.** The staged tree is built `FROM scratch` and carries `bin`, `sbin`, `lib`, `lib64`, `etc`, `tmp`, `usr`, `exaudf` and `build_info`. The Exasol UDF sandbox prepares mount points inside the SLC root before the client starts. When the root is writable it creates a missing directory and the gap stays invisible. Exasol Personal mounts each installed SLC read-only, so the same gap fails as `cannot create directories: Read-only file system`, reported to the user as `22002 VM crashed` (issue #110). Every official SLC is exported from a full distribution image and additionally creates `conf` and `buckets`, so no official SLC meets this failure. Exasol Personal applies the same read-only mount to a catalog SLC and to a custom one, through one code path, so the install method is not the difference. The missing skeleton is.

**Personal install path.** `container/personal-install-local` models one shape of a `local` backend: a managed VM with an SSH endpoint and a key at `local/node_access.pem`. Exasol Personal also runs a local deployment in a container engine on the invoking host. That deployment writes a descriptor with no SSH endpoint and provides no node key, so `scripts/install.sh --deployment` has no transport and fails on a raw `ssh` error. Exasol Personal ships `exasol slc install rust`, which downloads this project's release asset and installs it as a custom SLC, so the read-only defect above breaks the officially wired Rust route on that deployment shape.

- **Goals**: one build path that works on every distribution. An SLC that starts from a read-only root. A published compatibility claim that CI verifies. A clear failure message on a deployment shape this project cannot serve.
- **Non-Goals**: no change to the wire protocol, the SDK API, the ABI version or the runtime. No Exasol database on a macOS runner (see Consequences). No change to the glibc floor itself. No fix inside Exasol Personal or Nano.

### Decision

Publish what the container was built with, build against that record when the host cannot match it, and ship the directory skeleton the sandbox expects.

#### Architecture

```
  crates/cargo-exasol-udf/                  dist/
  ├── slc-glibc-floor.txt      ─┐           ├── slc-sandbox-skeleton.txt ─┐
  ├── slc-library-surface.txt  ─┤           └── supported-author-hosts.txt│
  └── slc-builder.txt          ─┤                        │               │
         (include_str!)         │                        │               │
                                ▼                        ▼               ▼
                        slc_surface.rs             Dockerfile      docs + drift test
                                │                        │
              ┌─────────────────┴────────┐               ▼
              ▼                          ▼          lc-rs.tar.gz
    build (--container,            validate              │
     platform warning)          (floor, identity,        ▼
              │                     surface)     dist/tests/slc_tarball_test.sh
              ▼
      target/slc/release/lib<crate>.so
              │
              ▼
   dist/tests/distro_udf_build_test.sh  ──▶  crates/it/tests/db_roundtrip.rs
        (7 distribution images)               (distro-built artifact runs)
```

#### Patterns

| Pattern | Where | Why |
|---------|-------|-----|
| One committed record per container fact, verified by the build that produces it | `slc-builder.txt`, `slc-sandbox-skeleton.txt` | The existing `slc-glibc-floor.txt` pattern: the value cannot drift from what ships, because the build fails when it does |
| Records the CLI reads live in the CLI crate; records only the packaging reads live in `dist/` | `crates/cargo-exasol-udf/slc-*.txt` versus `dist/*.txt` | The CLI compiles its records in with `include_str!`, which decides their location; nothing else does |
| One pure check shared by two subcommands | `slc_surface.rs` `platform_fit` | `build` and `validate` cannot disagree about one artifact, and the check unit-tests without an ELF fixture |
| Verify the claim against the recorded set, not against a hardcoded expectation | `dist/tests/distro_udf_build_test.sh` | A base-image refresh that raises a distribution's glibc fails the record, not a test assertion nobody reads |

### Consequences

| Decision | Alternatives Considered | Rationale |
|----------|------------------------|-----------|
| Ship the sandbox directory skeleton in the tarball | Ask Exasol Personal to mount the SLC writable; ship an OCI archive instead of a flat tarball | `exasol slc custom install` accepts a flat root-filesystem tarball only, and the read-only mount is shared with catalog SLCs, so neither alternative is reachable from this repository |
| Derive the skeleton from the official SLC top-level layout | Add only the three directories observed to fail | The observed set was found by iteration and has no known end; the official layout is a published set with a source |
| `build --container` as an explicit flag | Switch to a container build automatically when the host is unfit | An implicit container build changes build time and network use without the author asking; the warning names the flag instead |
| No `--builder-image` override | Accept an arbitrary builder image | The record is the contract; an operator behind a mirror retags the recorded reference locally, which needs no CLI surface |
| No Exasol E2E on a GitHub macOS runner | Colima or Lima on an Intel macOS runner | Three independent blockers: arm64 macOS runners have no nested virtualization, service containers are Linux-only on every macOS runner, and `exasol/docker-db` publishes amd64 images only. The Intel labels have an announced end of life and a 14 GB disk that a 3.2 GB compressed image plus a VM disk is not shown to fit |
| Fedora reaches the database through the container build | Run a Fedora host-built artifact end-to-end | A Fedora host build is above the glibc floor, so it cannot load; the container build is Fedora's supported path and is the one worth verifying end to end |

### Residual risk

The skeleton fix is evidence-led, not confirmed. Nano is closed source, and no Exasol document enumerates the sandbox mount set, so nothing proves that the recorded set is complete. The strongest available evidence is that an official SLC root contains that set and meets no such failure. The live check in Manual Testing is the only confirmation, and it needs a Podman Exasol Personal deployment, which CI cannot provide. If a failure persists past the recorded set, the remaining paths are added to the record and the defect is reported to Exasol Personal with the new path; the record and its enforcement do not change.

Two new privileges enter the build path, both deliberate.

- `build --container` mounts the author's Cargo workspace root into a container and runs a build there. That is the same trust boundary as a plain `cargo build`, which already runs arbitrary build scripts from the dependency graph.
- Task 4.1 mounts the runner's Docker socket into each distribution container so the in-container `--container` build reaches the host engine. A process with that socket is root-equivalent on the runner. This is confined to the `distro-build` job, which runs the repository's own committed harness against public base images and handles no secret. It must not be copied into a job that checks out untrusted code.

### Design depth

The change adds no module and no interface. `platform_fit` is a pure function in `slc_surface.rs`, the module that already owns every SLC platform fact for the CLI, so the one decision "does this artifact fit the container" gains a single owner instead of being duplicated across `build` and `validate`. It reads injected artifact facts and returns findings, so it performs no I/O and names no delivery mechanism. `build --container` is a new code path, not a new boundary: it produces the same artifact through the same subcommand, and the container engine stays behind `build`.

## Features

| Feature | Status | Spec |
|---------|--------|------|
| container/slc-platform-contract | CHANGED | `specs/_plans/add-cross-distro-compatibility/container/slc-platform-contract/spec.md` |
| container/slim-image | CHANGED | `specs/_plans/add-cross-distro-compatibility/container/slim-image/spec.md` |
| container/personal-install-local | CHANGED | `specs/_plans/add-cross-distro-compatibility/container/personal-install-local/spec.md` |
| tools/cargo-exaudf | CHANGED | `specs/_plans/add-cross-distro-compatibility/tools/cargo-exaudf/spec.md` |
| tools/cargo-exaudf-validate | CHANGED | `specs/_plans/add-cross-distro-compatibility/tools/cargo-exaudf-validate/spec.md` |

## Impact

Rust UDFs start working on an Exasol Personal local deployment, which is where issue #110 reports every UDF crashing with `22002 VM crashed`. Authors on a distribution whose glibc is above `2.41`, and authors on macOS, gain a supported build path through `cargo exasol-udf build --container`.

Two changes are breaking for existing users.

- `cargo exasol-udf validate` gains a new non-zero exit: an artifact whose rustc identity differs from the SLC builder's is now rejected. Such an artifact previously passed validation and then failed at `dlopen` with a fingerprint mismatch. A pipeline that validates an artifact built by a different rustc turns red at validation instead of at run time.
- `scripts/install.sh --deployment <name>` now exits non-zero on a local deployment with no SSH endpoint, instead of attempting `ssh` and failing there. The message names the two Personal CLI routes.

The container image and the CLI surface both change, so the workspace version needs a SemVer bump. That bump changes the ABI fingerprint and forces downstream UDF rebuilds, which is the existing rule in `CLAUDE.md`. The tarball grows by empty directory entries only, so the committed size ceiling is unaffected.

## Dependencies

- Exasol Personal `exasol slc custom install` and `exasol slc install rust` are the install routes named in the failure message. Both are Exasol Personal features, not additions here.
- The distribution matrix pulls `ubuntu:22.04`, `ubuntu:24.04`, `almalinux:8`, `almalinux:9`, `debian:13`, `opensuse/leap:16.0` and `fedora:44` from public registries.
- The container build path needs `docker` or `podman` on the author's host. Neither becomes a dependency of a plain host build.

## Implementation Tasks

1. **SLC platform records and the container build that enforces them**
   - [ ] 1.1 Add `dist/slc-sandbox-skeleton.txt`, one `<relative-path> <octal-mode>` per line, naming every directory the SLC root must provide that the staged tree does not already create. Derive the set from the top-level layout of an official SLC (a full distribution root plus `conf` and `buckets`), subtract what the staging stage already stages (`bin`, `sbin`, `lib`, `lib64`, `etc`, `tmp`, `usr`, `exaudf`, `build_info`), and include the three paths a read-only deployment failed on in order (`proc`, `var/tmp`, `buckets`). Record in the file's header comment which paths are observed failures and which come from the official layout. [expert]
   - [ ] 1.2 Add `dist/tests/skeleton_record_test.sh` with fixtures under `dist/fixtures/skeleton/`, rejecting an absolute path, a parent traversal, a duplicate entry and an unparseable mode.
   - [ ] 1.3 Stage the skeleton in the `Dockerfile` staging stage, reading the record rather than repeating the set. Create the directories after the usr-merge symlinks so a recorded path that collides with a reproduced symlink or a staged file fails the build, and keep every staged regular file under `/slc/usr`. Run the record validator in the stage so a malformed record fails the build. [expert]
   - [ ] 1.4 Add `slc_tarball_sandbox_skeleton_present` to `dist/tests/slc_tarball_test.sh`, asserting one empty directory entry with the recorded mode for every recorded path.
   - [ ] 1.5 Add `crates/cargo-exasol-udf/slc-builder.txt` with the builder image reference on the first line and the `rustc --version` identity that image provides on the second.
   - [ ] 1.6 Add `ARG BUILDER_IMAGE` to the `Dockerfile`, default it to the recorded reference, use it in the builder `FROM`, and fail the build when the stage's own `rustc --version` differs from the recorded identity or when the argument differs from the recorded reference.
   - [ ] 1.7 Add `slc_tarball_builder_identity_matches_record` to `dist/tests/slc_tarball_test.sh`, reading the baked fingerprint string out of the shipped `exaudf/exaudfclient` and comparing its rustc part to the record.
   - [ ] 1.8 Add `dist/supported-author-hosts.txt`, one `<family> <image-reference> <build-path>` per line, where the build path is `host` or `container`, covering Ubuntu, RHEL, Debian, openSUSE and Fedora.
   - [ ] 1.9 Rewrite the platform support matrix in `docs/installation.md` and the build-environment section of `docs/writing-a-udf.md` from that record, replacing "Rust 1.94+" with the exact toolchain requirement and naming `--container` for every container-only platform and for macOS.
   - [ ] 1.10 Add `dist/tests/supported_author_hosts_test.sh`, asserting the documentation names exactly the recorded platforms with the recorded build paths.

2. **CLI platform fit and the containerized build**
   - [ ] 2.1 Extend `crates/cargo-exasol-udf/src/slc_surface.rs` with the builder record and a pure `platform_fit` function that takes one artifact's glibc reference, rustc identity and `DT_NEEDED` set and returns the findings.
   - [ ] 2.2 Add `platform_fit` unit tests to `crates/cargo-exasol-udf/src/slc_surface_tests.rs` for a matching artifact, an above-floor artifact and a rustc-identity mismatch.
   - [ ] 2.3 Make `validate` report the artifact's rustc identity against the record, exit non-zero on a mismatch, and name `cargo exasol-udf build --container` in both the above-floor and the identity messages.
   - [ ] 2.4 Add `validate` integration tests in `crates/cargo-exasol-udf/tests/validate.rs` for the identity rejection and for the changed above-floor and summary output.
   - [ ] 2.5 Add `build --container`. Select `docker` first and `podman` second, error naming both when neither is present, mount the crate's Cargo workspace root at its own absolute path, run as the invoking uid and gid, mount the cargo registry cache, set the target directory to `target/slc`, and print `target/slc/release/lib<crate>.so`.
   - [ ] 2.6 Add `build` integration tests in `crates/cargo-exasol-udf/tests/build.rs` using a recording stub engine on `PATH` for argument assembly and engine preference, plus the missing-engine case with a scrubbed `PATH`.
   - [ ] 2.7 Make `build` run `platform_fit` on the produced artifact, print a warning naming each mismatching value and `--container`, and exit zero.
   - [ ] 2.8 Add a `build` integration test for the above-floor warning, reusing the stub-libc fixture technique already in `crates/cargo-exasol-udf/tests/validate.rs`.
   - [ ] 2.9 Document `--container` and `target/slc/release/lib<crate>.so` in `docs/cargo-ecosystem.md`.
   - [ ] 2.10 Add `dist/tests/distro_udf_build_test.sh <image-reference>`, which installs the pinned toolchain for that distribution family, scaffolds a UDF crate against the in-tree SDK, runs `build` and `validate`, asserts the outcome matches the recorded build path for that image, then runs `build --container` and asserts the artifact validates.

3. **Personal local deployments with no SSH transport**
   - [ ] 3.1 Detect a local deployment with no `connection.sshPort` or no `local/node_access.pem` in `scripts/install.sh`, before any copy step, and exit non-zero naming the missing fact, `exasol slc install rust`, `exasol slc custom install --source <tarball> --language rust` and the built tarball's path.
   - [ ] 3.2 Add `scripts/tests/install-personal-test.sh` cases for a descriptor without `connection.sshPort` and for an absent node key, asserting no `ssh` or `scp` runs.

4. **CI wiring and the distribution-built artifact end-to-end test**
   - [ ] 4.1 Add a `distro-build` job to `.github/workflows/ci.yml` with a matrix read from `dist/supported-author-hosts.txt`, running `dist/tests/distro_udf_build_test.sh` inside each image with the repository and the Docker socket mounted at their own host paths, so the in-container `--container` build reaches the host engine.
   - [ ] 4.2 Upload the openSUSE host-built artifact and the Fedora container-built artifact from that job.
   - [ ] 4.3 Pass `--build-arg BUILDER_IMAGE` from `crates/cargo-exasol-udf/slc-builder.txt` in the `build-slc` job, and run `dist/tests/skeleton_record_test.sh` and `dist/tests/supported_author_hosts_test.sh` in the `unit-tests` job.
   - [ ] 4.4 Download both artifacts in the `integration` job and add `distro_built_artifact_executes` to `crates/it/tests/db_roundtrip.rs`, registering each under its own BucketFS path and asserting the scalar result.
   - [ ] 4.5 Record the distribution-matrix wiring rule in `CLAUDE.md`, beside the existing `test-udfs/*` allowlist rule.

## Parallelization

| Group | Tasks | Depends on | Knowledge |
|-------|-------|------------|-----------|
| A: SLC platform records and container build | 1.1-1.10 | — | spec deltas `container/slc-platform-contract`, `container/slim-image`; `Dockerfile`, `dist/slc-sandbox-skeleton.txt`, `dist/supported-author-hosts.txt`, `crates/cargo-exasol-udf/slc-builder.txt`, `dist/tests/slc_tarball_test.sh`, `dist/tests/skeleton_record_test.sh`, `dist/tests/supported_author_hosts_test.sh`, `dist/fixtures/skeleton/`, `docs/installation.md`, `docs/writing-a-udf.md` |
| B: CLI platform fit and containerized build | 2.1-2.10 | A (reads `crates/cargo-exasol-udf/slc-builder.txt`) | spec deltas `tools/cargo-exaudf`, `tools/cargo-exaudf-validate`; `crates/cargo-exasol-udf/src/slc_surface.rs`, `crates/cargo-exasol-udf/src/slc_surface_tests.rs`, `crates/cargo-exasol-udf/src/build.rs`, `crates/cargo-exasol-udf/src/validate.rs`, `crates/cargo-exasol-udf/src/main.rs`, `crates/cargo-exasol-udf/tests/build.rs`, `crates/cargo-exasol-udf/tests/validate.rs`, `dist/tests/distro_udf_build_test.sh`, `docs/cargo-ecosystem.md` |
| C: Personal local transport gap | 3.1-3.2 | — | spec delta `container/personal-install-local`; `scripts/install.sh`, `scripts/tests/install-personal-test.sh` |
| D: CI wiring and distribution artifact E2E | 4.1-4.5 | A, B | verification harness for the group A and group B scenarios; `.github/workflows/ci.yml`, `crates/it/tests/db_roundtrip.rs`, `CLAUDE.md` |

Group D owns every edit to `.github/workflows/ci.yml`. Groups A and B add their tests and records but wire none of them into CI, so no two groups write that file.

## Dead Code Removal

| Type | Location | Reason |
|------|----------|--------|
| None | — | This plan adds records, one CLI flag and one detection branch. No module, function or test becomes unreachable. The superseded above-floor message text in `crates/cargo-exasol-udf/src/validate.rs` is rewritten in place, not left behind. |

## Verification

### Scenario Coverage

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| cargo-exaudf: build --container produces an artifact the SLC accepts from any host | Integration | `dist/tests/distro_udf_build_test.sh` | `container_build_artifact_validates` |
| cargo-exaudf: build --container reports a missing container engine | Integration | `crates/cargo-exasol-udf/tests/build.rs` | `build_container_without_engine_errors` |
| cargo-exaudf: build --container reports a missing container engine (engine preference) | Integration | `crates/cargo-exasol-udf/tests/build.rs` | `build_container_prefers_docker_over_podman` |
| cargo-exaudf: build --container produces an artifact the SLC accepts (argument assembly) | Integration | `crates/cargo-exasol-udf/tests/build.rs` | `build_container_invokes_engine_with_expected_arguments` |
| cargo-exaudf: build warns when the host cannot produce an artifact the SLC accepts | Integration | `crates/cargo-exasol-udf/tests/build.rs` | `build_warns_when_artifact_exceeds_slc_floor` |
| cargo-exaudf: build warns when the host cannot produce an artifact the SLC accepts (silent on a fit host) | Integration | `crates/cargo-exasol-udf/tests/build.rs` | `build_is_silent_when_artifact_matches_the_record` |
| cargo-exaudf-validate: validate accepts a compatible .so | Integration | `crates/cargo-exasol-udf/tests/validate.rs` | `validate_accepts_named_entries_and_reports_platform_summary` |
| cargo-exaudf-validate: validate rejects an artifact above the SLC glibc floor | Integration | `crates/cargo-exasol-udf/tests/validate.rs` | `validate_rejects_glibc_above_floor` |
| cargo-exaudf-validate: validate rejects an artifact built by a rustc the SLC does not run | Integration | `crates/cargo-exasol-udf/tests/validate.rs` | `validate_rejects_rustc_identity_mismatch` |
| cargo-exaudf-validate: validate rejects an artifact built by a rustc the SLC does not run (record is the source) | Unit | `crates/cargo-exasol-udf/src/slc_surface_tests.rs` | `platform_fit_reads_the_rustc_identity_from_the_builder_record` |
| slc-platform-contract: Extracted tree provides the sandbox directory skeleton | Integration | `dist/tests/slc_tarball_test.sh` | `slc_tarball_sandbox_skeleton_present` |
| slc-platform-contract: SLC publishes the builder identity its loader enforces | Integration | `dist/tests/slc_tarball_test.sh` | `slc_tarball_builder_identity_matches_record` |
| slc-platform-contract: Published platform support names one build path per author host | Integration | `dist/tests/supported_author_hosts_test.sh` | `supported_author_hosts_documentation_matches_record` |
| slc-platform-contract: Published platform support names one build path per author host (observed path matches record) | Integration | `dist/tests/distro_udf_build_test.sh` | `recorded_build_path_matches_observed_outcome` |
| slim-image: Staging creates the sandbox directory skeleton | Integration | `dist/tests/slc_tarball_test.sh` | `slc_tarball_sandbox_skeleton_modes_and_emptiness` |
| slim-image: Staging creates the sandbox directory skeleton (malformed record fails) | Integration | `dist/tests/skeleton_record_test.sh` | `skeleton_record_rejects_absolute_traversal_duplicate_and_bad_mode` |
| slim-image: Builder toolchain and glibc runtime | Integration | `dist/tests/slc_tarball_test.sh` | `slc_tarball_builder_identity_matches_record` |
| personal-install-local: A local deployment with no SSH transport is reported with the supported route | Integration | `scripts/tests/install-personal-test.sh` | `local_install_without_ssh_port_names_the_personal_routes` |
| personal-install-local: A local deployment with no SSH transport is reported with the supported route (absent node key) | Integration | `scripts/tests/install-personal-test.sh` | `local_install_without_node_key_names_the_personal_routes` |
| Cross-cutting: a distribution-built artifact executes against a live database | Integration | `crates/it/tests/db_roundtrip.rs` | `distro_built_artifact_executes` |

### Manual Testing

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| container/slc-platform-contract | `docker build --target artifact --output type=local,dest=/tmp/lc-out . && tar tzvf /tmp/lc-out/lc-rs.tar.gz \| grep -E ' \./(proc\|buckets\|conf\|var/tmp)/$'` | Four directory entries listed, each with size 0 |
| container/slim-image | `bash dist/tests/slc_tarball_test.sh /tmp/lc-out/lc-rs.tar.gz` | `All tests passed` |
| tools/cargo-exaudf | `cargo exasol-udf new /tmp/probe && cd /tmp/probe && cargo exasol-udf build --container` | Prints `target/slc/release/libprobe.so`, and the file exists and is owned by the invoking user |
| tools/cargo-exaudf-validate | `cargo exasol-udf validate /tmp/probe/target/slc/release/libprobe.so` | Reports the glibc reference against `2.41`, the rustc identity against the record, and exits 0 |
| container/personal-install-local | `scripts/install.sh --deployment <a Linux local deployment>` | Exits non-zero, names the missing SSH fact, `exasol slc install rust`, `exasol slc custom install --source <path> --language rust`, and the built tarball path; no `ssh` or `scp` runs |
| Cross-cutting (issue #110) | Install the built tarball with `exasol slc custom install --source lc-rs.tar.gz --alias myrust --language rust` on a Podman Exasol Personal deployment, then `SELECT health_check()` | Returns `ok` instead of `22002 VM crashed` |

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Build | `cargo build --release` | Exit 0 |
| Unit tests | `cargo test` | 0 failures |
| Shell gates | `bash dist/tests/skeleton_record_test.sh && bash dist/tests/supported_author_hosts_test.sh && bash scripts/tests/install-personal-test.sh` | 0 failures |
| SLC build | `docker build --target artifact --output type=local,dest=/tmp/lc-out .` | Exit 0 |
| Tarball contract | `bash dist/tests/slc_tarball_test.sh /tmp/lc-out/lc-rs.tar.gz` | `All tests passed` |
| Distribution matrix | `for i in $(awk '{print $2}' dist/supported-author-hosts.txt); do bash dist/tests/distro_udf_build_test.sh "$i"; done` | 0 failures |
| Integration tests | `cargo test -p it --features integration` | 0 failures |
| Lint | `cargo clippy --workspace --all-targets --all-features -- -D warnings` | 0 warnings |
| Format | `cargo fmt --all -- --check` | No changes |
