# Plan: add-cross-distro-compatibility

## Summary

Make a Rust UDF buildable on every platform an author works on, make the SLC runnable on a deployment that mounts it read-only, and prove that a UDF executes when Exasol itself runs on each supported Linux distribution. Six committed records carry the contract, one CLI flag builds against it, and two matrices verify the two halves of the claim.

## Design

### Context

"The Rust SLC must be compatible with all Linux supported by Exasol" has two readings, and both are in scope. An author must be able to build a UDF on those distributions. A UDF must also run when Exasol itself is installed on one of them. Decision [13] records that reconciliation and the evidence for it. The claim fails today in four independent places, each with its own mechanism.

**Author host.** `cargo exasol-udf build` runs a host `cargo build --release`. That binds the artifact to two host properties. The linker stamps glibc symbol versions from the host glibc, and the SDK bakes the host's `rustc --version` string into the vtable fingerprint. The container accepts an artifact only when the first is at or below its floor of `2.41` and the second equals the rustc that compiled the shipped `exaudfclient`. A distribution whose glibc is above the floor has no host build path at all. A distribution-packaged Rust toolchain never matches the fingerprint, whatever its version number. `validate` catches neither case for the rustc identity, because it compares an artifact's whole fingerprint against the CLI's own, and the CLI was compiled by the same host rustc that built the artifact.

**SLC root filesystem.** The staged tree is built `FROM scratch` and carries `bin`, `sbin`, `lib`, `lib64`, `etc`, `tmp`, `usr`, `exaudf` and `build_info`. The Exasol UDF sandbox prepares mount points inside the SLC root before the client starts. When the root is writable it creates a missing directory and the gap stays invisible. Exasol Personal mounts each installed SLC read-only, so the same gap fails as `cannot create directories: Read-only file system`, reported to the user as `22002 VM crashed` (issue #110). Every official SLC is exported from a full distribution image and additionally creates `conf` and `buckets`, so no official SLC meets this failure. Exasol Personal applies the same read-only mount to a catalog SLC and to a custom one, through one code path, so the install method is not the difference. The missing skeleton is.

**Personal install path.** `container/personal-install-local` models one shape of a `local` backend: a managed VM with an SSH endpoint and a key at `local/node_access.pem`. Exasol Personal also runs a local deployment in a container engine on the invoking host. That deployment writes a descriptor with no SSH endpoint and provides no node key, so `scripts/install.sh --deployment` has no transport and fails on a raw `ssh` error. Exasol Personal ships `exasol slc install rust`, which downloads this project's release asset and installs it as a custom SLC, so the read-only defect above breaks the officially wired Rust route on that deployment shape.

**Database host kernel.** The engine does not execute a UDF in the database process. It launches `nschroot`, which creates an unprivileged user namespace and needs `CAP_SYS_ADMIN` inside it to build the sandbox. The host kernel's security module grants or denies that capability. AppArmor enforces it on Debian, Ubuntu and openSUSE, and SELinux enforces it on RHEL and Fedora. Nothing the SLC image carries changes the outcome, and a denial reaches the user as `22002 VM crashed` with the cause only in the host audit log.

This project already meets that failure on its own Ubuntu CI hosts and works around it with a documented `sysctl`, recorded in `CLAUDE.md`. No comparable evidence exists for any other family, because a distribution's container on an Ubuntu-kernel runner exercises the runner's module and never that distribution's own.

- **Goals**: one build path that works on every author platform. An SLC that starts from a read-only root. A published author-host claim and a published database-host claim, each verified by the evidence that fits it. One end-to-end run per database-host family, performed on a host of that family. A clear failure message on a deployment shape this project cannot serve.
- **Non-Goals**: no change to the wire protocol, the SDK API, the ABI version or the runtime. No Exasol database on a macOS runner (see Consequences). No change to the glibc floor itself. No fix inside Exasol Personal or Nano. No AppArmor profile or SELinux policy module authored here: the plan publishes the host prerequisite and ships no policy. No provisioning of the runners the database-host matrix needs, which is infrastructure outside this repository (see Dependencies).

### Decision

Publish what the container was built with, build against that record when the host cannot match it, ship the directory skeleton the sandbox expects, and publish the database-host claim as its own record proved by its own runs.

#### Architecture

```
Author-build path

  crates/cargo-exasol-udf/                dist/
  ├── slc-glibc-floor.txt     ─┐          ├── slc-sandbox-skeleton.txt ──┐
  ├── slc-library-surface.txt ─┤          └── supported-author-hosts.txt ┤
  └── slc-builder.txt         ─┤                       │                 │
        (include_str!)         ▼                       ▼                 ▼
                        slc_surface.rs            Dockerfile      docs + drift tests
                               │                       │
             ┌─────────────────┴────────────┐          ▼
             ▼                              ▼     lc-rs.tar.gz
   build (--container,                  validate       │
    platform warning)          (floor, surface,        ▼
             │                  SDK part vs CLI,   dist/tests/slc_tarball_test.sh
             │                  rustc part vs record)
             ▼
   target/slc/release/lib<crate>.so
             │
             ▼
   dist/tests/distro_udf_build_test.sh   (executed inside each distribution image)
             │
             ▼
   crates/it/tests/db_roundtrip.rs       (a distribution-built artifact runs)

Host-runtime path

   dist/supported-database-hosts.txt ──▶ docs + drift test
             │
             ▼
   .github/actions/exasol-it            (one bring-up sequence, one prerequisite source)
             │
             ├── ubuntu ........... the existing integration job, hosted runner
             └── debian, opensuse, rhel, fedora ... one runner per family
                         │
                         ▼
             lc-rs.tar.gz registered, a scalar UDF returns instead of 22002
```

#### Patterns

| Pattern | Where | Why |
|---------|-------|-----|
| One committed record per container fact, verified by the build that produces it | `slc-builder.txt`, `slc-sandbox-skeleton.txt` | The existing `slc-glibc-floor.txt` pattern: the value cannot drift from what ships, because the build fails when it does |
| One committed record per published claim, verified by the run that proves it | `supported-author-hosts.txt`, `supported-database-hosts.txt` | A claim checked against a record is a claim a base-image refresh or a new kernel default can falsify, instead of a sentence in a document nobody re-reads |
| Record location follows which component reads the record | `crates/cargo-exasol-udf/slc-*.txt` versus `dist/*.txt` | The CLI compiles its records in with `include_str!`, which decides their location; nothing else does |
| One pure check shared by two subcommands | `slc_surface.rs` `platform_fit` | `build` and `validate` cannot disagree about one artifact, and the check unit-tests without an ELF fixture |
| One owner per format decision | the SDK owns the fingerprint string, and the builder record holds its rustc part verbatim | The sanitize-and-truncate rule stays in `crates/exasol-udf-sdk/build.rs` alone, so the Dockerfile, the tarball test and the CLI compare strings instead of re-deriving one |
| One extraction rule for the baked fingerprint | `strings -a <binary> \| grep -E '^[0-9]+\.[0-9]+\.[0-9]+:'`, which MUST match exactly one line | `EXA_SDK_FINGERPRINT` lives in read-only data with no symbol of its own, so three components must recover it identically. The builder stage, the tarball test and the record author run this one command, and a zero-match or multi-match result fails the step that ran it instead of silently comparing the wrong string |
| One bring-up sequence for every live-database job | `.github/actions/exasol-it` | Comparing a UDF's outcome across host kernels is only meaningful when every host runs the identical sequence |

### Consequences

| Decision | Alternatives Considered | Rationale |
|----------|------------------------|-----------|
| Both readings of the compatibility request are in scope | Author-build compatibility only; database-host compatibility only | The two readings describe different failures with different mechanisms, and the request excludes neither. Decision [13] records the reconciliation |
| Ship the sandbox directory skeleton in the tarball | Ask Exasol Personal to mount the SLC writable; ship an OCI archive instead of a flat tarball | `exasol slc custom install` accepts a flat root-filesystem tarball only, and the read-only mount is shared with catalog SLCs, so neither alternative is reachable from this repository |
| Derive the skeleton from the official SLC top-level layout | Add only the three directories observed to fail | The observed set was found by iteration and has no known end; the official layout is a published set with a source |
| `build --container` as an explicit flag | Switch to a container build automatically when the host is unfit | An implicit container build changes build time and network use without the author asking; the warning names the flag instead |
| No `--builder-image` override | Accept an arbitrary builder image | The record is the contract; an operator behind a mirror retags the recorded reference locally, which needs no CLI surface |
| Compare the fingerprint in two parts against two sources | Compare the whole fingerprint against the CLI, as today; relax the comparison to a version prefix | The two parts have different owners. The SDK version belongs to the CLI's own crate, the rustc identity belongs to the container. One whole-string comparison against the CLI rejects every container-built artifact on a host whose rustc differs, which is the exact host the flag exists for |
| A run on a host of each family is the only evidence for the database-host claim | A distribution container on a hosted Ubuntu runner; a virtual machine per family started inside a hosted runner | The enforcing security module lives in the host kernel, so a container names a distribution without exercising its kernel. GitHub hosts Ubuntu, Windows and macOS runners only, so no hosted label carries a SUSE, Fedora or RHEL kernel |
| The host prerequisite is a committed record | Keep the `sysctl` inline in the workflow, as today | The Ubuntu workaround is already an operator prerequisite living only in a CI comment. One record makes every family's prerequisite publishable, checkable and applied identically on every host |
| `distro-build` runs on every push and `integration` waits for it | Run it on `main` and a schedule; run the distribution artifacts in a separate live-database job | The distribution-built artifacts are the evidence for the cross-cutting scenario, so gating them behind a schedule would let a red matrix reach `main`. The legs run in parallel, so the added wall clock is one leg, not seven |
| No Exasol E2E on a GitHub macOS runner | Colima or Lima on an Intel macOS runner | Three independent blockers: arm64 macOS runners have no nested virtualization, service containers are Linux-only on every macOS runner, and `exasol/docker-db` publishes amd64 images only. The Intel labels have an announced end of life and a 14 GB disk that a 3.2 GB compressed image plus a VM disk is not shown to fit |
| Fedora reaches the database through the container build | Run a Fedora host-built artifact end-to-end | A Fedora host build is above the glibc floor, so it cannot load; the container build is Fedora's supported path and is the one worth verifying end to end |

### Residual risk

The skeleton fix is evidence-led, not confirmed. Nano is closed source, and no Exasol document enumerates the sandbox mount set, so nothing proves that the recorded set is complete. The strongest available evidence is that an official SLC root contains that set and meets no such failure. The live check in Manual Testing is the only confirmation, and it needs a Podman Exasol Personal deployment, which CI cannot provide. If a failure persists past the recorded set, the remaining paths are added to the record and the defect is reported to Exasol Personal with the new path. The record and its enforcement do not change.

No live-database leg mounts the SLC root read-only. Every leg extracts the tarball into writable BucketFS storage, so a missing mount point would still be created there and stay invisible. The read-only chroot self-test of task 1.9 is the only automated read-only evidence this repository can produce, and it runs `exaudfclient` directly rather than through `nschroot`, so it proves that the staged tree starts without write access and not that the mount set is complete. Completeness of the mount set stays confirmed by the Manual Testing row alone.

The database-host matrix depends on runners this repository does not own. A family whose runner is unregistered or offline fails its leg rather than skipping it, which is the project's existing rule for a test that needs infrastructure. That makes missing evidence visible as a red leg instead of a silently narrower claim.

The database-host claim covers the versions its record names and no others. `it-runner` is compiled on `ubuntu-latest` and downloaded onto each runner, so a family version whose glibc is below that floor cannot run the harness and carries no leg. RHEL 8 and RHEL 9, openSUSE Leap 15 and Debian 12 are below it. The record names the version each family's claim covers, so an operator reads a verified version rather than a family name that overstates the evidence.

The SELinux prerequisite is not yet known. AppArmor's is: the `sysctl` this project already applies on Ubuntu. What RHEL and Fedora need, if anything, is established by the first run on an enforcing host of each. The record is where that answer lands, and the record's drift test is what stops it from staying inside a job step where no operator reads it.

Two new privileges enter the build path, both deliberate.

- `build --container` mounts the author's Cargo workspace root into a container and runs a build there. That is the same trust boundary as a plain `cargo build`, which already runs arbitrary build scripts from the dependency graph.
- The `distro-build` job mounts the runner's Docker socket into each distribution container so the in-container `--container` build reaches the host engine. A process with that socket is root-equivalent on the runner. This is confined to that job, which runs the repository's own committed harness against public base images and handles no secret. It must not be copied into a job that checks out untrusted code.

### Design depth

The change adds no module to the crates and one feature to the spec library. `platform_fit` is a pure function in `slc_surface.rs`, the module that already owns every SLC platform fact for the CLI, so the one decision "does this artifact fit the container" gains a single owner instead of being duplicated across `build` and `validate`. It reads injected artifact facts and returns findings, so it performs no I/O and names no delivery mechanism. `build --container` is a new code path, not a new boundary: it produces the same artifact through the same subcommand, and the container engine stays behind `build`.

`container/host-runtime-compatibility` is a separate feature rather than three more scenarios on `container/slc-platform-contract`, because the two answer different questions from different evidence. The platform contract states what the image guarantees, and every one of its scenarios is checked by inspecting the image. The host-runtime feature states what the database host must provide, and its scenarios are checked only by running a UDF on such a host. Folding them together would give one feature two unrelated reasons to change and would invite exactly the conflation that round 1 caught, where a build path is published as if it proved a UDF runs.

The fingerprint format keeps one owner. `crates/exasol-udf-sdk/build.rs` decides how a rustc version string becomes a fingerprint part, and every other component compares strings it did not derive. The builder record holds that part verbatim, read out of the compiled binary, so the Dockerfile, the tarball test and the CLI never re-implement the sanitize-and-truncate rule.

## Features

| Feature | Status | Spec |
|---------|--------|------|
| container/slc-platform-contract | CHANGED | `specs/_plans/add-cross-distro-compatibility/container/slc-platform-contract/spec.md` |
| container/slim-image | CHANGED | `specs/_plans/add-cross-distro-compatibility/container/slim-image/spec.md` |
| container/host-runtime-compatibility | NEW | `specs/_plans/add-cross-distro-compatibility/container/host-runtime-compatibility/spec.md` |
| container/personal-install | CHANGED | `specs/_plans/add-cross-distro-compatibility/container/personal-install/spec.md` |
| container/personal-install-local | CHANGED | `specs/_plans/add-cross-distro-compatibility/container/personal-install-local/spec.md` |
| tools/cargo-exaudf | CHANGED | `specs/_plans/add-cross-distro-compatibility/tools/cargo-exaudf/spec.md` |
| tools/cargo-exaudf-validate | CHANGED | `specs/_plans/add-cross-distro-compatibility/tools/cargo-exaudf-validate/spec.md` |

## Impact

Rust UDFs start working on an Exasol Personal local deployment, which is where issue #110 reports every UDF crashing with `22002 VM crashed`. Authors on a distribution whose glibc is above `2.41`, and authors on macOS, gain a supported build path through `cargo exasol-udf build --container`. The published compatibility claim splits in two: an author-host claim proved by building on each platform, and a database-host claim proved by running a UDF on a host of each family. Operators gain the host prerequisite each family needs, which until now existed only as a comment in this project's own workflow.

Group 5 removes a concrete failure of its own. On an Exasol Personal local deployment backed by a container engine, `scripts/install.sh --deployment <name>` today attempts SSH against a deployment that publishes no SSH endpoint and fails on a raw `ssh` connection error naming neither the deployment shape nor a route that works. That is a different command from the `exasol slc custom install` route issue #110 reports, and a second way the same deployment shape is unserved today.

Two changes are breaking for existing users.

- `cargo exasol-udf validate` gains a new non-zero exit: an artifact whose rustc identity differs from the SLC builder's is now rejected. Such an artifact previously passed validation and then failed at `dlopen` with a fingerprint mismatch. A pipeline that validates an artifact built by a different rustc turns red at validation instead of at run time. The same change makes `validate` accept a container-built artifact it previously rejected on a host whose own rustc differs, which is the case the flag exists for.
- `scripts/install.sh --deployment <name>` now exits non-zero on a local deployment with no SSH endpoint, instead of attempting `ssh` and failing there. The message names the two Personal CLI routes.

The container image and the CLI surface both change, so the workspace version needs a SemVer bump. That bump changes the ABI fingerprint and forces downstream UDF rebuilds, which is the existing rule in `CLAUDE.md`. The tarball grows by empty directory entries only, so the committed size ceiling is unaffected.

## Dependencies

- Exasol Personal `exasol slc custom install` and `exasol slc install rust` are the install routes named in the failure message. Both are Exasol Personal features, not additions here.
- The author-host distribution matrix pulls `ubuntu:22.04`, `ubuntu:24.04`, `almalinux:8`, `almalinux:9`, `debian:13`, `opensuse/leap:16.0` and `fedora:44` from public registries. Each image is one row of `dist/supported-author-hosts.txt`, keyed by `<family> <version>`, so the two Ubuntu rows and the two RHEL rows stay distinct rows.
- The container build path needs `docker` or `podman` on the author's host. Neither becomes a dependency of a plain host build.
- The database-host matrix needs one self-hosted GitHub Actions runner per non-Ubuntu family, each carrying the label triple `self-hosted, linux, <family>` for `debian`, `opensuse`, `rhel` and `fedora`, each running that distribution as its host operating system with a container engine available. Provisioning, registering and maintaining those runners is infrastructure work outside this repository's diff. This plan's workflow assumes only that a runner carrying the label exists and is reachable, and names no endpoint, credential or account. The Ubuntu family needs no new runner: the existing `integration` job on the hosted `ubuntu-latest` label is that family's leg.
- Every `host-runtime` runner MUST provide four host facts the workflow cannot install for it.
  - A glibc at or above the glibc of the `ubuntu-latest` runner that compiles `it-runner`. That binary is glibc-dynamic and is downloaded onto the runner rather than rebuilt there, so a runner below that floor fails at the loader with `GLIBC_x.y not found` before any database starts. Task 6.4 compares the two versions first and fails naming both.
  - Passwordless `sudo`, because the recorded host prerequisite of `dist/supported-database-hosts.txt` is applied as a host command.
  - Free disk for the `exasol/docker-db` image, whose compressed size is 3.2 GB, plus its data volume. The composite action reclaims no disk of its own, unlike the hosted `integration` job, which frees about 30 GB before pulling that image.
  - Memory for the 4 GiB the database container is pinned to, plus the container engine's own overhead.
- The database-host record names one version per family, and a runner MUST run the version its family's row records. Task 4.1 names versions whose glibc clears the floor above, so an older in-support version of a family carries no automated leg. Residual risk states what that leaves unverified.
- `distro-build` becomes a prerequisite of `integration` and of `host-runtime`, because both consume the two artifacts that job uploads. Its seven legs run in parallel and each carries a cargo cache keyed on its family and version, so the job adds roughly one leg's wall clock ahead of `integration` rather than seven. Each leg carries `timeout-minutes: 45`, which bounds the worst case a stuck leg can add to the pipeline.

## Implementation Tasks

1. **SLC platform records and the container build that enforces them**
   - [ ] 1.1 Add `dist/slc-sandbox-skeleton.txt`, one `<relative-path> <octal-mode>` per line, naming every directory the SLC root must provide that the staged tree does not already create. Derive the set from the top-level layout of an official SLC (a full distribution root plus `conf` and `buckets`), subtract what the staging stage already stages (`bin`, `sbin`, `lib`, `lib64`, `etc`, `tmp`, `usr`, `exaudf`, `build_info`), and include the three paths a read-only deployment failed on in order (`proc`, `var/tmp`, `buckets`). Sort the file by path depth, and record every parent of a nested entry as its own earlier line, so `var` precedes `var/tmp`. Record in the file's header comment which paths are observed failures and which come from the official layout. [expert]
   - [ ] 1.2 Add `dist/tests/skeleton_record_test.sh` with fixtures under `dist/tests/fixtures/skeleton/`, the directory every existing fixture set in this repository already uses, rejecting an absolute path, a parent traversal, a duplicate entry, an unparseable mode and an entry whose parent is neither recorded on an earlier line nor staged by the stage itself.
   - [ ] 1.3 Stage the skeleton in the `Dockerfile` staging stage, reading the record rather than repeating the set. Create each entry with a plain `mkdir` and never `mkdir -p`, in recorded order, so an entry whose parent is missing fails the build instead of being created silently. Create the directories after the usr-merge symlinks so a recorded path that collides with a reproduced symlink or a staged file fails the build, and keep every staged regular file under `/slc/usr`. Run the record validator in the stage so a malformed record fails the build. [expert]
   - [ ] 1.4 Add two cases to `dist/tests/slc_tarball_test.sh`. `slc_tarball_sandbox_skeleton_present` asserts one directory entry for every recorded path. `slc_tarball_sandbox_skeleton_modes_and_emptiness` asserts that each of those entries carries the recorded octal mode and contains no member beneath it.
   - [ ] 1.5 Add `crates/cargo-exasol-udf/slc-builder.txt` with three lines: the builder image reference, the rustc-identity part of the fingerprint the SDK bakes into the compiled `exaudfclient`, and the exact toolchain version that image provides, in `<major>.<minor>.<patch>` form. Take the second line verbatim from the compiled binary with the extraction rule in § Patterns, rather than re-deriving it from `rustc --version`, so `crates/exasol-udf-sdk/build.rs` stays the only owner of that string's format.
   - [ ] 1.6 In the `Dockerfile` builder stage, after `exaudfclient` is compiled, recover its baked fingerprint string with the extraction rule in § Patterns, split it at the first colon, and fail the build when the rustc-identity part differs from the record's second line. Pin the builder `FROM` to the exact patch version the record's third line names, keep it a literal reference and add no `ARG BUILDER_IMAGE`, because an `ARG` before `FROM` cannot take its default from the record and would duplicate the literal instead of replacing it.
   - [ ] 1.7 Add `slc_tarball_builder_identity_matches_record` to `dist/tests/slc_tarball_test.sh`, recovering the baked fingerprint string from the shipped `exaudf/exaudfclient` with the extraction rule in § Patterns, splitting it at the first colon and comparing the rustc-identity part to the record's second line as a plain string equality.
   - [ ] 1.8 Add `dist/tests/builder_record_test.sh` with `builder_record_matches_dockerfile_from`, asserting the `Dockerfile`'s builder `FROM` reference equals the record's first line, that the first line's tag carries the toolchain version the third line names, and that the record holds exactly three non-empty lines.
   - [ ] 1.9 Run the `Dockerfile` staging stage's chroot self-test a second time against an unwritable staged tree, after the tar step so the recorded modes in the tarball stay untouched. Use `chmod -R a-w /slc` and `chroot --userspec=` an unprivileged donor user, assert the same wrong-argument-count error and non-zero exit, and fail the build otherwise. This is the only automated read-only evidence available here, because the self-test runs `exaudfclient` directly rather than through `nschroot`.

2. **CLI platform fit and the containerized build**
   - [ ] 2.1 Extend `crates/cargo-exasol-udf/src/slc_surface.rs` with the builder record and a pure `platform_fit` function that takes one artifact's glibc reference, rustc identity and `DT_NEEDED` set and returns the findings.
   - [ ] 2.2 Split the fingerprint comparison in `crates/cargo-exasol-udf/src/validate.rs` (`runtime_fingerprint()` at line 146). Compare the artifact's fingerprint in two parts: the SDK-version part against the CLI's own `EXA_SDK_FINGERPRINT`, and the rustc-identity part against the second line of `crates/cargo-exasol-udf/slc-builder.txt`. Report which part mismatched. Never compare the fingerprint as one whole string against the CLI's own value, because that rejects every container-built artifact on a host whose rustc differs from the recorded builder rustc, which is the host the flag exists for. [expert]
   - [ ] 2.3 Add three `platform_fit` unit cases to `crates/cargo-exasol-udf/src/slc_surface_tests.rs`: `platform_fit_accepts_an_artifact_matching_both_records` for a matching artifact, `platform_fit_reports_a_glibc_above_the_floor` for an above-floor artifact, and `platform_fit_reads_the_rustc_identity_from_the_builder_record` for a rustc-identity mismatch, which asserts the compared value comes from the builder record and not from the CLI's own baked fingerprint.
   - [ ] 2.4 Make `validate` report the artifact's rustc identity against the record, exit non-zero on a mismatch, and name `cargo exasol-udf build --container` in both the above-floor and the identity messages.
   - [ ] 2.5 Add `validate` integration tests in `crates/cargo-exasol-udf/tests/validate.rs` for the identity rejection, the SDK-version-part rejection, the changed above-floor and summary output, and for a container-built artifact whose rustc identity equals the record but differs from the rustc that compiled the CLI, which must exit zero.
   - [ ] 2.6 Add `build --container`. Select `docker` first and `podman` second, error naming both when neither is present, mount the crate's Cargo workspace root at its own absolute path, run as the invoking uid and gid, mount the cargo registry cache, set the target directory to `target/slc`, and print `target/slc/release/lib<crate>.so` followed by a line stating that the artifact satisfies the recorded glibc floor and builder identity by construction. Run no local `validate` step, so the path also serves a host that cannot `dlopen` a Linux `.so`.
   - [ ] 2.7 Add `build` integration tests in `crates/cargo-exasol-udf/tests/build.rs` using a recording stub engine on `PATH` for argument assembly and engine preference, the missing-engine case with a scrubbed `PATH`, and `build_container_skips_local_validation_and_states_why`, which asserts the by-construction line is printed and that no validation is invoked.
   - [ ] 2.8 Make `build` run `platform_fit` on the produced artifact, print a warning naming each mismatching value and `--container`, and exit zero.
   - [ ] 2.9 Add two `build` integration tests to `crates/cargo-exasol-udf/tests/build.rs`, reusing the stub-libc fixture technique already in `crates/cargo-exasol-udf/tests/validate.rs`. `build_warns_when_artifact_exceeds_slc_floor` covers the above-floor warning. `build_is_silent_when_artifact_matches_the_record` asserts that an artifact matching both records prints no warning and exits zero.
   - [ ] 2.10 Document `--container` and `target/slc/release/lib<crate>.so` in `docs/cargo-ecosystem.md`, stating that the artifact takes the invoking host's architecture and therefore matches the SLC built for that architecture, and that `validate` needs a Linux host while `--container` does not.
   - [ ] 2.11 Add `dist/tests/distro_udf_build_test.sh <family> <version>`, taking the two fields that key one row of `dist/supported-author-hosts.txt`, because a family alone cannot separate `ubuntu 22.04` from `ubuntu 24.04`. The script assumes it is already running inside a container of that row's image (see task 6.1, which starts the container and execs it). Take the repository path from the mount, install rustup and the exact toolchain version the builder record's third line names (never the distribution's own Rust package), build `cargo-exasol-udf` from the mounted workspace with that toolchain, scaffold the probe crate under the mounted repository path and point `CARGO_HOME` at a directory under that same path, so every path the harness creates is identical on the host and inside the container and a sibling container started through the mounted Docker socket sees it. Run `build` and `validate`, assert the outcome matches the build path that row records, then run `build --container` and assert the artifact validates. Skip no row: a row whose image-reference field holds the placeholder `-` contributes no leg at all.

3. **Published author-host support**
   - [ ] 3.1 Add `dist/supported-author-hosts.txt`, one line per platform version with exactly five space-separated fields: `<family> <version> <image-reference> <glibc-version> <build-path>`, where the build path is `host` or `container`. The row key is the pair `<family> <version>`, and every consumer uses that pair: the harness argument of task 2.11, the cargo cache key of task 6.1 and the drift test of task 6.1. Record one row per matrix image, which gives `ubuntu 22.04`, `ubuntu 24.04`, `rhel 8`, `rhel 9`, `debian 13`, `opensuse 16.0` and `fedora 44`, the two RHEL rows carrying the `almalinux:8` and `almalinux:9` images decision [7] chose. Add macOS with the placeholder `-` in the version, image-reference and glibc fields and the build path `container`. Record in the header that `host` means both a glibc at or below the floor and a rustup-installed toolchain of the exact version the builder record's third line names, because a distribution-packaged Rust never produces the recorded identity.
   - [ ] 3.2 Rewrite the platform support matrix in `docs/installation.md` and the build-environment section of `docs/writing-a-udf.md` from that record, replacing "Rust 1.94+" with the exact toolchain requirement and its rustup install route, and naming `--container` for every container-only platform including macOS.
   - [ ] 3.3 Add `dist/tests/supported_author_hosts_test.sh` with `supported_author_hosts_documentation_matches_record`, asserting the documentation names exactly the recorded rows by their `<family> <version>` key with the recorded build paths, and `imageless_platform_is_container_only`, asserting every row whose image-reference field is the placeholder `-` carries the build path `container`. Both cases read only the record and the documentation, so neither waits on the CI wiring of group F.

4. **Published database-host support**
   - [ ] 4.1 Add `dist/supported-database-hosts.txt`, one line per family version with exactly four space-separated fields: `<family> <version> <kernel-security-module> <host-prerequisite>`, where the module is `apparmor` or `selinux` and the prerequisite is `none` or the exact host command a UDF needs before it can start. The row key is the pair `<family> <version>`, matching the author-host record's key, because the prerequisite is version-specific rather than family-wide. Cover `ubuntu 24.04`, `debian 13`, `opensuse 16.0`, `rhel 10` and `fedora 44`, each a version whose glibc clears the `it-runner` floor § Dependencies states. The `ubuntu 24.04` row carries the `sysctl kernel.apparmor_restrict_unprivileged_userns=0` command `CLAUDE.md` already documents, which becomes that command's single source, and which is an Ubuntu 24.04 kernel setting rather than one every AppArmor family accepts.
   - [ ] 4.2 Add a database-host support statement to `docs/installation.md`, written from that record and kept separate from the author-host statement of task 3.2, naming per family version the kernel security module, the prerequisite, and the `apparmor="DENIED" ... comm="nschroot" capability=21` audit signature that identifies a host-kernel denial behind a bare `22002 VM crashed`. State that a version the record does not name is unverified rather than unsupported.
   - [ ] 4.3 Add `dist/tests/supported_database_hosts_test.sh` with `supported_database_hosts_documentation_matches_record`, asserting the documentation names exactly the recorded rows by their `<family> <version>` key with their recorded module, prerequisite and audit signature, and `author_and_database_host_claims_are_stated_separately`, asserting the two documentation statements are distinct sections and that neither cites the other's record. Both cases read only the records and the documentation, so neither waits on the CI wiring of group F.

5. **Personal local deployments with no SSH transport**
   - [ ] 5.1 Detect a local deployment with no `connection.sshPort` or no `local/node_access.pem` in `scripts/install.sh`, before any copy step, and exit non-zero naming the missing fact, `exasol slc install rust`, `exasol slc custom install --source <tarball> --language rust` and the built tarball's path.
   - [ ] 5.2 Add `scripts/tests/install-personal-test.sh` cases for a descriptor without `connection.sshPort` and for an absent node key, asserting no `ssh` or `scp` runs.
   - [ ] 5.3 Add `local_backend_with_ssh_facts_still_selects_the_ssh_transport` to `scripts/tests/install-personal-test.sh`, asserting a local deployment carrying both `connection.sshPort` and `local/node_access.pem` still enters the SSH/filesystem transport, so the new detection narrows the `local` branch rather than removing it.

6. **CI wiring and end-to-end evidence**
   - [ ] 6.1 Add a `distro-build` job to `.github/workflows/ci.yml` whose matrix is a literal list written into the workflow, one entry per row of `dist/supported-author-hosts.txt` whose image-reference field is not the placeholder `-`, each entry carrying the family, the version and the image. Use no generator job and no `fromJSON`, because GitHub Actions evaluates `strategy.matrix` before any step of the job runs, so a matrix derived from a repository file would need a prior job. The drift test below is what keeps the literal list equal to the record. Each leg runs on every push, carries `timeout-minutes: 45` and a cargo cache keyed on `<family>-<version>`, and starts the distribution container itself with `docker run -v "$PWD:$PWD" -v /var/run/docker.sock:/var/run/docker.sock -w "$PWD" <image>`, then execs `bash dist/tests/distro_udf_build_test.sh <family> <version>` inside it. The repository path is identical inside and outside the container, so the sibling container the in-container `--container` build starts through the mounted socket resolves the same paths. Add `distro_build_matrix_matches_record` to `dist/tests/supported_author_hosts_test.sh`, asserting the literal matrix entries and the record's non-placeholder rows are the same set of `<family> <version>` pairs carrying the same images.
   - [ ] 6.2 Upload the openSUSE host-built artifact and the Fedora container-built artifact from that job, scaffolded under the two distinct crate names `distro_probe_opensuse` and `distro_probe_fedora`, so the two downloaded `.so` files do not collide on disk in the consuming job.
   - [ ] 6.3 Run `dist/tests/skeleton_record_test.sh`, `dist/tests/builder_record_test.sh`, `dist/tests/supported_author_hosts_test.sh` and `dist/tests/supported_database_hosts_test.sh` in the `unit-tests` job. Pass no builder build argument from the `build-slc` job, since task 1.6 keeps the literal `FROM` and task 1.8 checks it against the record.
   - [ ] 6.4 Add `.github/actions/exasol-it/action.yml`, a composite action taking a database-host family, that family's recorded version and an Exasol version. Its first step compares the host's own glibc, read from `ldd --version`, against the floor the `build` job records beside the `it-runner` artifact, and fails naming both versions when the host is below it, so a runner short of the floor reports the cause instead of a raw `GLIBC_x.y not found` from the loader. Extend the `build` job to write that floor as a one-line file and upload it with the `it-test` artifact, so the floor tracks the runner that actually compiled the binary. The action then applies exactly the prerequisite `dist/supported-database-hosts.txt` records for that family and version, read from the checked-out working tree, and makes no other host change. It expects the calling job to have already downloaded one artifact set: the SLC tarball, the UDF `.so` artifacts, the IT test binary and the two distribution-built artifacts of task 6.2, because `db_roundtrip_all_scenarios` reads all of them on every leg. It starts one `exasol/docker-db` container with the existing memory and shm settings, waits with `exapump wait`, extracts the BucketFS write password and runs the prebuilt `it-runner`. It reclaims no disk of its own, unlike the hosted `integration` job, because a self-hosted runner carries no preinstalled hosted-runner software to remove. Every live-database job then runs one identical sequence, which is what makes an outcome comparable across host kernels.
   - [ ] 6.5 Replace the `integration` job's inline `sysctl`, container start, wait, password and test steps with that action, passing family `ubuntu` and its recorded version, and comment the job as the Ubuntu leg of the database-host matrix. Add an `actions/checkout` step before the action, because the local reference `./.github/actions/exasol-it` and the record the action reads both resolve from the runner workspace, and this job checks nothing out today. Add `distro-build` to its `needs`, download both distribution-built artifacts, and add `distro_built_artifact_executes` to `crates/it/tests/db_roundtrip.rs` as a helper function called from `db_roundtrip_all_scenarios` that reuses that test's existing `Harness`, registering each artifact under its own BucketFS path and asserting the scalar result.
   - [ ] 6.6 Add a `host-runtime` job to `.github/workflows/ci.yml` whose matrix is a literal list written into the workflow, one entry per row of `dist/supported-database-hosts.txt` except the `ubuntu` row, each entry carrying the family and that row's recorded version, and each leg running `runs-on: [self-hosted, linux, <family>]`. Use no generator job and no `fromJSON`, for the reason task 6.1 states. Give the job `needs: [build, distro-build]`, add an `actions/checkout` step first, then download the SLC tarball, the UDF `.so` artifacts, the IT test binary and the two distribution-built artifacts of task 6.2, which is the same artifact set the `integration` job downloads and which `db_roundtrip_all_scenarios` reads on every leg. Then call the composite action with the leg's family, its recorded version and the lowest pinned Exasol version. A leg whose runner is unreachable or whose container engine is absent fails, and never skips. Add `host_runtime_matrix_matches_record` to `dist/tests/supported_database_hosts_test.sh`, asserting the literal matrix entries plus the `integration` job's Ubuntu leg are exactly the record's `<family> <version>` rows.
   - [ ] 6.7 Record in `CLAUDE.md`, beside the existing `test-udfs/*` allowlist rule and the AppArmor note: the distribution-matrix wiring rule, the family-to-runner-label map for the `host-runtime` job, and that the AppArmor `sysctl` now lives in `dist/supported-database-hosts.txt` rather than inline in the workflow.

## Parallelization

| Group | Tasks | Depends on | Knowledge |
|-------|-------|------------|-----------|
| A: SLC platform records and container build | 1.1-1.9 | — | spec deltas `container/slc-platform-contract`, `container/slim-image`; `Dockerfile`, `dist/slc-sandbox-skeleton.txt`, `crates/cargo-exasol-udf/slc-builder.txt`, `dist/tests/slc_tarball_test.sh`, `dist/tests/skeleton_record_test.sh`, `dist/tests/builder_record_test.sh`, `dist/tests/fixtures/skeleton/` |
| B: CLI platform fit and containerized build | 2.1-2.11 | A (reads `crates/cargo-exasol-udf/slc-builder.txt`) | spec deltas `tools/cargo-exaudf`, `tools/cargo-exaudf-validate`; `crates/cargo-exasol-udf/src/slc_surface.rs`, `crates/cargo-exasol-udf/src/slc_surface_tests.rs`, `crates/cargo-exasol-udf/src/build.rs`, `crates/cargo-exasol-udf/src/validate.rs`, `crates/cargo-exasol-udf/src/main.rs`, `crates/cargo-exasol-udf/tests/build.rs`, `crates/cargo-exasol-udf/tests/validate.rs`, `dist/tests/distro_udf_build_test.sh`, `docs/cargo-ecosystem.md` |
| C: Published author-host support | 3.1-3.3 | B (documents `--container`, the surface B implements) | spec delta `container/slc-platform-contract` scenario "Published platform support names one build path per author host"; `dist/supported-author-hosts.txt`, `dist/tests/supported_author_hosts_test.sh`, `docs/installation.md`, `docs/writing-a-udf.md` |
| D: Published database-host support | 4.1-4.3 | C (writes the other half of `docs/installation.md`) | spec delta `container/host-runtime-compatibility`; `dist/supported-database-hosts.txt`, `dist/tests/supported_database_hosts_test.sh`, `docs/installation.md` |
| E: Personal local transport gap | 5.1-5.3 | — | spec deltas `container/personal-install`, `container/personal-install-local`; `scripts/install.sh`, `scripts/tests/install-personal-test.sh` |
| F: CI wiring and end-to-end evidence | 6.1-6.7 | A, B, C, D | verification harness for the group A, B, C and D scenarios; `.github/workflows/ci.yml`, `.github/actions/exasol-it/action.yml`, `crates/it/tests/db_roundtrip.rs`, `dist/tests/supported_author_hosts_test.sh`, `dist/tests/supported_database_hosts_test.sh`, `CLAUDE.md` |

Group F owns every edit to `.github/workflows/ci.yml`. Groups A through E add their records, code and tests but wire none of them into CI, so no two groups write that file.

Three file overlaps are sequenced rather than shared. Groups C and D both write `docs/installation.md`, in two sections task 4.3 asserts are distinct, so D declares a dependency on C instead of racing it. Group F appends one matrix-agreement case to each of the two drift-test scripts C and D create, which is why F depends on both: those cases read the workflow file F owns, so they cannot pass before F lands. Tasks 1.1 to 1.4 and 1.5 to 1.9 stay in one group because both halves write the `Dockerfile` and `dist/tests/slc_tarball_test.sh`, which is a consolidation signal rather than a parallelism opportunity.

## Dead Code Removal

| Type | Location | Reason |
|------|----------|--------|
| Superseded comparison | `crates/cargo-exasol-udf/src/validate.rs` whole-fingerprint equality at line 146 | Task 2.2 replaces it with the two-part comparison. The old branch is rewritten in place, not left beside the new one |
| Superseded CI step | `.github/workflows/ci.yml` inline `sysctl kernel.apparmor_restrict_unprivileged_userns=0` step and its comment block | Task 6.5 moves the command into `dist/supported-database-hosts.txt` and the explanation into `docs/installation.md`, so the inline copy would be a second source |

## Verification

### Scenario Coverage

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| cargo-exaudf: build --container produces an artifact the SLC accepts from any host | Integration | `dist/tests/distro_udf_build_test.sh` | `container_build_artifact_validates` |
| cargo-exaudf: build --container produces an artifact the SLC accepts (argument assembly) | Integration | `crates/cargo-exasol-udf/tests/build.rs` | `build_container_invokes_engine_with_expected_arguments` |
| cargo-exaudf: build --container reports a missing container engine | Integration | `crates/cargo-exasol-udf/tests/build.rs` | `build_container_without_engine_errors` |
| cargo-exaudf: build --container reports a missing container engine (engine preference) | Integration | `crates/cargo-exasol-udf/tests/build.rs` | `build_container_prefers_docker_over_podman` |
| cargo-exaudf: build --container serves a host that cannot load the artifact it produces | Integration | `crates/cargo-exasol-udf/tests/build.rs` | `build_container_skips_local_validation_and_states_why` |
| cargo-exaudf: build warns when the host cannot produce an artifact the SLC accepts | Integration | `crates/cargo-exasol-udf/tests/build.rs` | `build_warns_when_artifact_exceeds_slc_floor` |
| cargo-exaudf: build warns when the host cannot produce an artifact the SLC accepts (silent on a fit host) | Integration | `crates/cargo-exasol-udf/tests/build.rs` | `build_is_silent_when_artifact_matches_the_record` |
| cargo-exaudf-validate: validate accepts a compatible .so | Integration | `crates/cargo-exasol-udf/tests/validate.rs` | `validate_accepts_named_entries_and_reports_platform_summary` |
| cargo-exaudf-validate: validate accepts a compatible .so (container-built artifact under a foreign host rustc) | Integration | `crates/cargo-exasol-udf/tests/validate.rs` | `validate_accepts_container_built_artifact_under_a_different_host_rustc` |
| cargo-exaudf-validate: validate rejects an ABI or fingerprint mismatch | Integration | `crates/cargo-exasol-udf/tests/validate.rs` | `validate_rejects_sdk_version_part_mismatch` |
| cargo-exaudf-validate: validate rejects an artifact above the SLC glibc floor | Integration | `crates/cargo-exasol-udf/tests/validate.rs` | `validate_rejects_glibc_above_floor` |
| cargo-exaudf-validate: validate rejects an artifact built by a rustc the SLC does not run | Integration | `crates/cargo-exasol-udf/tests/validate.rs` | `validate_rejects_rustc_identity_mismatch` |
| cargo-exaudf-validate: validate rejects an artifact built by a rustc the SLC does not run (record is the source) | Unit | `crates/cargo-exasol-udf/src/slc_surface_tests.rs` | `platform_fit_reads_the_rustc_identity_from_the_builder_record` |
| slc-platform-contract: Extracted tree provides the sandbox directory skeleton | Integration | `dist/tests/slc_tarball_test.sh` | `slc_tarball_sandbox_skeleton_present` |
| slc-platform-contract: SLC publishes the builder identity its loader enforces | Integration | `dist/tests/slc_tarball_test.sh` | `slc_tarball_builder_identity_matches_record` |
| slc-platform-contract: SLC publishes the builder identity its loader enforces (recorded reference matches the build definition) | Integration | `dist/tests/builder_record_test.sh` | `builder_record_matches_dockerfile_from` |
| slc-platform-contract: Published platform support names one build path per author host | Integration | `dist/tests/supported_author_hosts_test.sh` | `supported_author_hosts_documentation_matches_record` |
| slc-platform-contract: Published platform support names one build path per author host (imageless platform is container-only) | Integration | `dist/tests/supported_author_hosts_test.sh` | `imageless_platform_is_container_only` |
| slc-platform-contract: Published platform support names one build path per author host (imageless platform contributes no leg) | Integration | `dist/tests/supported_author_hosts_test.sh` | `distro_build_matrix_matches_record` |
| slc-platform-contract: Published platform support names one build path per author host (observed path matches record) | Integration | `dist/tests/distro_udf_build_test.sh` | `recorded_build_path_matches_observed_outcome` |
| slim-image: Staging creates the sandbox directory skeleton | Integration | `dist/tests/slc_tarball_test.sh` | `slc_tarball_sandbox_skeleton_modes_and_emptiness` |
| slim-image: Staging creates the sandbox directory skeleton (malformed record fails) | Integration | `dist/tests/skeleton_record_test.sh` | `skeleton_record_rejects_absolute_traversal_duplicate_bad_mode_and_orphan_parent` |
| slim-image: Builder toolchain and glibc runtime | Integration | `dist/tests/builder_record_test.sh` | `builder_record_matches_dockerfile_from` |
| slim-image: Staged tree passes an in-build chroot self-test (unwritable staged tree) | Integration | `Dockerfile` staging stage | `chroot_self_test_on_a_read_only_tree`, a named build step whose failure fails `docker build --target artifact` |
| host-runtime-compatibility: A registered SLC executes a UDF on every supported database-host kernel | Integration | `crates/it/tests/db_roundtrip.rs` | `db_roundtrip_all_scenarios`, run once per recorded family by the `host-runtime` job and by the `integration` job's Ubuntu leg |
| host-runtime-compatibility: The host prerequisite is a committed record, not a step buried in the harness | Integration | `dist/tests/supported_database_hosts_test.sh` | `host_runtime_matrix_matches_record` |
| host-runtime-compatibility: The host prerequisite is a committed record (documentation names module, prerequisite and audit signature) | Integration | `dist/tests/supported_database_hosts_test.sh` | `supported_database_hosts_documentation_matches_record` |
| host-runtime-compatibility: Author-host and database-host support are published as two separate claims | Integration | `dist/tests/supported_database_hosts_test.sh` | `author_and_database_host_claims_are_stated_separately` |
| personal-install: Deployment backend selects the transport | Integration | `scripts/tests/install-personal-test.sh` | `local_backend_with_ssh_facts_still_selects_the_ssh_transport` |
| personal-install-local: A local deployment with no SSH transport is reported with the supported route | Integration | `scripts/tests/install-personal-test.sh` | `local_install_without_ssh_port_names_the_personal_routes` |
| personal-install-local: A local deployment with no SSH transport is reported with the supported route (absent node key) | Integration | `scripts/tests/install-personal-test.sh` | `local_install_without_node_key_names_the_personal_routes` |
| Cross-cutting: a distribution-built artifact executes against a live database | Integration | `crates/it/tests/db_roundtrip.rs` | `distro_built_artifact_executes` |

### Manual Testing

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| container/slc-platform-contract | `docker build --target artifact --output type=local,dest=/tmp/lc-out . && tar tzvf /tmp/lc-out/lc-rs.tar.gz \| grep -E ' \./(proc\|buckets\|conf\|var/tmp)/$'` | Four directory entries listed, each with size 0 |
| container/slim-image | `bash dist/tests/slc_tarball_test.sh /tmp/lc-out/lc-rs.tar.gz` | `All tests passed` |
| tools/cargo-exaudf | `cargo exasol-udf new /tmp/probe && cd /tmp/probe && cargo exasol-udf build --container` | Prints `target/slc/release/libprobe.so` and the by-construction line, and the file exists and is owned by the invoking user |
| tools/cargo-exaudf (macOS) | The same two commands on a macOS host with Docker Desktop or Podman | Prints the same two lines, produces the Linux `.so`, and invokes no validation step |
| tools/cargo-exaudf-validate | `cargo exasol-udf validate /tmp/probe/target/slc/release/libprobe.so` | Reports the glibc reference against `2.41`, the rustc identity against the record, and exits 0 |
| container/personal-install-local | `scripts/install.sh --deployment <a Linux local deployment>` | Exits non-zero, names the missing SSH fact, `exasol slc install rust`, `exasol slc custom install --source <path> --language rust`, and the built tarball path; no `ssh` or `scp` runs |
| container/host-runtime-compatibility (debian 13) | On a Debian 13 host: apply the recorded prerequisite, start `exasol/docker-db`, register the SLC and run `SELECT health_check()` | Returns `ok` instead of `22002 VM crashed` |
| container/host-runtime-compatibility (opensuse 16.0) | The same sequence on an openSUSE Leap 16.0 host | Returns `ok` instead of `22002 VM crashed` |
| container/host-runtime-compatibility (rhel 10) | The same sequence on a RHEL 10 host with SELinux enforcing | Returns `ok` instead of `22002 VM crashed`. Any host change needed beyond the recorded prerequisite is added to the record |
| container/host-runtime-compatibility (fedora 44) | The same sequence on a Fedora 44 host with SELinux enforcing | Returns `ok` instead of `22002 VM crashed`. Any host change needed beyond the recorded prerequisite is added to the record |
| Cross-cutting (issue #110) | Install the built tarball with `exasol slc custom install --source lc-rs.tar.gz --alias myrust --language rust` on a Podman Exasol Personal deployment, then `SELECT health_check()` | Returns `ok` instead of `22002 VM crashed` |

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Build | `cargo build --release` | Exit 0 |
| Unit tests | `cargo test` | 0 failures |
| Shell gates | `bash dist/tests/skeleton_record_test.sh && bash dist/tests/builder_record_test.sh && bash dist/tests/supported_author_hosts_test.sh && bash dist/tests/supported_database_hosts_test.sh && bash scripts/tests/install-personal-test.sh` | 0 failures |
| SLC build | `docker build --target artifact --output type=local,dest=/tmp/lc-out .` | Exit 0 |
| Tarball contract | `bash dist/tests/slc_tarball_test.sh /tmp/lc-out/lc-rs.tar.gz` | `All tests passed` |
| Author-host matrix | `awk '!/^#/ && $3 != "-" {print $1, $2, $3}' dist/supported-author-hosts.txt \| while read -r fam ver img; do docker run --rm -v "$PWD:$PWD" -v /var/run/docker.sock:/var/run/docker.sock -w "$PWD" "$img" bash dist/tests/distro_udf_build_test.sh "$fam" "$ver"; done` | 0 failures |
| Database-host matrix | On a host of each family version `dist/supported-database-hosts.txt` names: apply that row's recorded prerequisite, then `cargo test -p it --features integration` | 0 failures on every row. An unavailable row is a failure, not a skip |
| Integration tests | `cargo test -p it --features integration` | 0 failures |
| Lint | `cargo clippy --workspace --all-targets --all-features -- -D warnings` | 0 warnings |
| Format | `cargo fmt --all -- --check` | No changes |
