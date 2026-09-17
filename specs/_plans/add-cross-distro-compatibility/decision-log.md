# Decision Log: add-cross-distro-compatibility

## Interview

Planning ran headless. One asynchronous exchange with the human took place, through the orchestrator.

**Q:** Issue #110 (a Podman custom-SLC install crash on Exasol Personal) looks unrelated to the E2E and distribution-compatibility request. How should planning proceed?
**A:** Both are related, plan for both.

**Q (implied by the request text, answered by research rather than by the human):** Can an end-to-end test run on a GitHub Actions macOS runner?
**A:** Resolved by research, not escalated. See decision [8].

The remaining request text, taken verbatim from the user: "I want an E2E for Suse and Fedora as well as the Rust SLC must be compatible with all Linux supported by Exasol. Do a research first; also: can we do an E2E on macOS via GH actions?"

## Design Decisions

### [1] The sandbox directory skeleton, not the install method, is the root cause of issue #110

- **Decision:** Fix issue #110 in this repository by shipping the directory skeleton the Exasol UDF sandbox expects inside the SLC root. Treat the reporter's `podman import` versus `podman pull` hypothesis as disproved.
- **Alternatives:** Ship an OCI archive so the custom install can use `podman load`. Rejected: `exasol slc custom install` validates and accepts a flat root-filesystem tarball only, so no other format reaches that code path. Ask Exasol Personal to mount the SLC writable. Rejected: `internal/localinstall/podman_install.go` attaches every SLC with `--mount type=image`, which is read-only by default, through one code path shared by catalog and custom SLCs, so the mount mode is not what distinguishes a working SLC.
- **Rationale:** Evidence, in three parts. Every official flavor Dockerfile in `exasol/script-languages-release` runs `mkdir /conf /buckets` on top of a full Ubuntu base, so an official SLC root already contains every directory the sandbox creates. `podman import` produces an ordinary image and a container from it gets a writable layer exactly as a pulled image does, so the import path removes no writability. The reporter's own progression `proc` then `var/tmp` then `buckets` converges on that official layout rather than diverging. This project's `FROM scratch` tree is the outlier.
- **Promotes to ADR:** yes

### [2] Derive the skeleton from the official SLC layout, not from the observed failures

- **Decision:** `dist/slc-sandbox-skeleton.txt` names every top-level directory an official SLC root provides, plus `conf`, `buckets` and `var/tmp`. The file's header records which entries are observed failures and which come from the official layout.
- **Alternatives:** Add only `proc`, `var/tmp` and `buckets`, the three paths observed to fail. Rejected: that set was found one failure at a time and has no known end, because Nano is closed source and no Exasol document enumerates the mount set.
- **Rationale:** An empty directory costs no bytes and no library surface, so matching a published layout is cheaper than another round of iteration on a user's deployment.
- **Promotes to ADR:** no

### [3] The SLC publishes its builder rustc identity as a committed record

- **Decision:** Add `crates/cargo-exasol-udf/slc-builder.txt` holding the builder image reference and the `rustc --version` identity it provides. The container build verifies both against its own builder stage. `validate` compares an artifact's baked identity against the record.
- **Alternatives:** Keep comparing an artifact against the CLI's own `EXA_SDK_FINGERPRINT`. Rejected: `cargo install cargo-exasol-udf` bakes the author's rustc into the CLI, so that comparison passes on every host and detects no skew against the container. Relax the runtime's fingerprint check to a version prefix. Rejected: the check exists to stop toolchain-mismatch undefined behaviour, and loosening it trades a clear error for silent risk.
- **Rationale:** The loader compares the fingerprint by exact string equality, so an artifact loads only when its rustc identity equals the SLC builder's. Without a published record, that requirement is unknowable to an author and undetectable before `dlopen`. This is the same pattern `slc-glibc-floor.txt` already uses: one committed value, verified by the build that produces it.
- **Promotes to ADR:** yes

### [4] `cargo exasol-udf build --container` is the portable build path

- **Decision:** Add an explicit `--container` flag that runs the release build inside the recorded builder image, writing to `target/slc/release/`.
- **Alternatives:** Switch to a container build automatically when the host is unfit. Rejected: an implicit container build changes build time, disk use and network use without the author asking, and it fails on a host with no engine that would otherwise have produced a usable artifact. Add a `cargo-zigbuild` dependency to target a fixed glibc version. Rejected: it fixes the glibc half only, and the rustc identity half would still fail at load. Document a `docker run` recipe instead of a flag. Rejected: the CLI already owns the build, so handing the author a recipe leaves the deepest part of the job outside the tool.
- **Rationale:** Only a build inside the recorded image satisfies both the glibc floor and the rustc identity by construction. One flag on the subcommand that already owns building keeps the common path unchanged.
- **Promotes to ADR:** yes

### [5] No `--builder-image` override

- **Decision:** `--container` reads the builder image reference from the record and accepts no override.
- **Alternatives:** Expose `--builder-image <ref>` for mirrored or air-gapped registries. Rejected for now.
- **Rationale:** The record is the contract, and a flag that lets an author build against a different image reintroduces exactly the mismatch the flag exists to prevent. An operator behind a mirror retags the recorded reference locally, which needs no CLI surface.
- **Promotes to ADR:** no

### [6] Records the CLI reads live in the CLI crate; records only the packaging reads live in `dist/`

- **Decision:** `slc-builder.txt` joins `slc-glibc-floor.txt` and `slc-library-surface.txt` in `crates/cargo-exasol-udf/`. `slc-sandbox-skeleton.txt` and `supported-author-hosts.txt` go in `dist/`.
- **Alternatives:** One `slc-platform.toml` holding every value. Rejected: the `debian:trixie-slim` staging stage has no TOML parser and reads these records with `cat` and a shell loop, so a structured format would need a parser inside a stage that deliberately carries none. Put every record in one directory. Rejected: it would either place a file the CLI never reads inside the CLI crate, or move a file out of the `include_str!` path it depends on.
- **Rationale:** The CLI compiles its records in with `include_str!`, which decides where they live. Nothing else imposes a location, so the packaging records sit with the packaging tests that read them.
- **Promotes to ADR:** no

### [7] The distribution matrix covers seven images, chosen from Exasol's own published requirements

- **Decision:** Verify `ubuntu:22.04`, `ubuntu:24.04`, `almalinux:8`, `almalinux:9`, `debian:13`, `opensuse/leap:16.0` and `fedora:44`.
- **Alternatives:** Verify only openSUSE and Fedora, the two families named in the request. Rejected: the request's second half asks for compatibility with every Linux Exasol supports, and neither openSUSE nor Fedora appears on Exasol's database-host list. Verify every in-support version of every family. Rejected: twelve or more legs for no new failure mode, since the glibc version is what varies and the families repeat it.
- **Rationale:** Exasol's system requirements page names Ubuntu 20.04, 22.04 and 24.04 LTS and RHEL 8, 9 and 10 as supported database hosts, and the driver pages additionally name openSUSE 15 and Debian 11 and 12. AlmaLinux stands in for RHEL, which needs no subscription to pull. Ubuntu 20.04 is excluded because it is in extended security maintenance. Fedora is on no Exasol list but is the request's explicit ask and the only mainstream image whose glibc is above the floor, which makes it the canary for the container-only build path. Debian 13 sits exactly at the floor and is the SLC's own donor.
- **Promotes to ADR:** no

### [8] No Exasol end-to-end test on a GitHub Actions macOS runner

- **Decision:** Do not add a macOS end-to-end job. Document why, and make macOS a supported author host through `--container` instead.
- **Alternatives:** Colima or Lima on an Intel macOS label (`macos-15-intel`, `macos-15-large`). Rejected. A self-hosted Intel Mac runner, or a third-party macOS provider with nested virtualization. Rejected as disproportionate to the coverage gained.
- **Rationale:** Three independent blockers, any one of them decisive. GitHub documents that arm64 macOS runners support no nested virtualization, because of Apple's Virtualization Framework, so every Docker, Colima, Lima and Podman path fails there. GitHub documents that job and service containers require a Linux runner, on every macOS label. `exasol/docker-db` publishes amd64 manifests only and states that only Docker on Linux is supported, so an arm64 runner could not run the database even with nested virtualization. The remaining Intel labels have an announced end of life, are billed even on public repositories, and carry a 14 GB disk that a 3.2 GB compressed image plus a Linux VM disk plus the database's data volume is not shown to fit. No cross-job networking exists that would let a macOS job reach a database in a Linux job. The useful macOS outcome is the author path, not the server path: a macOS host cannot emit a Linux `.so` natively, and `--container` gives it one.
- **Promotes to ADR:** yes

### [9] The Fedora end-to-end path is the container-built artifact

- **Decision:** Run the openSUSE host-built artifact and the Fedora container-built artifact through the live-database suite.
- **Alternatives:** Run a Fedora host-built artifact end to end. Rejected: a Fedora host build references glibc symbol versions above the floor, so it cannot load, and asserting that it loads would encode a failure.
- **Rationale:** An end-to-end test should exercise the path a Fedora author actually uses. That path is `--container`. The host-built Fedora artifact is still covered, as the case whose rejection message must name the remedy.
- **Promotes to ADR:** no

### [10] Test the real container build in the distribution harness, not in a Rust integration test

- **Decision:** `crates/cargo-exasol-udf/tests/build.rs` asserts engine selection and argument assembly against a recording stub engine on `PATH`. `dist/tests/distro_udf_build_test.sh` runs the real container build on every recorded image.
- **Alternatives:** A Rust integration test that runs a real `docker run`. Rejected: `cargo test` would then fail on a developer machine with no container engine, and the project's rule is that a test needing a container fails rather than skips, which would make the default test command unusable off CI.
- **Rationale:** The stub gives deterministic, fast coverage of the argument contract. The shell harness gives real coverage on seven distributions, which is stronger evidence than one local run.
- **Promotes to ADR:** no

### [11] Group D owns every CI file edit

- **Decision:** Groups A, B and C add records, code and tests but wire nothing into `.github/workflows/ci.yml`. Group D does all CI wiring and the live-database scenario, and depends on A and B.
- **Alternatives:** Let each group wire its own CI job. Rejected: three groups writing one file is a merge conflict, not parallelism.
- **Rationale:** The workflow file is a single shared resource with no natural split, so it gets a single owner and a declared dependency instead.
- **Promotes to ADR:** no

### [12] `scripts/install.sh` detects the SSH-less local deployment by descriptor, not by host operating system

- **Decision:** Treat a local deployment as SSH-less when `deployment.json` carries no `connection.sshPort` or `local/node_access.pem` is absent.
- **Alternatives:** Branch on the host operating system, treating Linux as the container-engine shape and macOS as the VM shape. Rejected: Exasol Personal's own release notes state that macOS now runs the same Podman installation inside a managed VM, so the host operating system does not determine the deployment shape and would become wrong without warning.
- **Rationale:** The descriptor is the only fact available on every host, and Exasol Personal's own specification states that a local deployment's `connection` block carries no SSH endpoint.
- **Promotes to ADR:** no

## Review Findings
