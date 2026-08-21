# Feature: slim-image

Packages the `exaudfclient` binary into a slim, Debian-staged SLC root filesystem (Option A only, no Rust toolchain) that Exasol can register as a `localzmq+protobuf` language container.

## Background

The SLC is a three-stage build from a single root `Dockerfile`. A `rust:1.94-trixie` builder compiles `exaudfclient` with zmq statically linked (no `libzmq3-dev` — `zmq-sys` falls back to `zeromq-src`). `exaudfclient` links no bzip2 at all: `exarrow-rs`'s bzip2 usage lives entirely behind its CSV `IMPORT`/`EXPORT` local-file-compression feature, a code path the client's `ExaConnection` usage never reaches, so the linker drops the dependency. The builder also derives the two architecture-dependent values — the Debian multiarch triplet and the built binary's own `PT_INTERP` loader path — and records them for the next stage, because the runtime donor image carries neither `binutils` nor `dpkg-architecture`. A `debian:trixie-slim` stage is then both donor and packager: it reproduces its own usr-merge symlink layout inside a staged `/slc` tree, copies the glibc runtime, the dlopen-only NSS/resolver modules and the documented UDF library surface out of itself with `cp -L`, adds the binary, the language-definition file and the notice bundles, and tars `/slc` into `lc-rs.tar.gz`. A final `FROM scratch` artifact stage exposes the tarball for `docker build --output`. Nothing outside that curated set ships: the staged tree carries no shell, no package manager, no coreutils, no Rust toolchain and no vendored Cargo registry, so it supports precompiled `.so` UDFs only. Every architecture-dependent path is derived rather than hardcoded, so a native build on x86_64 or aarch64 produces the matching-architecture SLC with no cross-compilation.

The staged tree is the UDF's entire root filesystem at run time, so what it provides beyond the client's own link closure is a deliberate, documented contract rather than an accident of `cp -L`. That contract — the fixed library surface and the glibc version floor it publishes to authors — is specified in `container/slc-platform-contract`, not here; this feature covers only the build mechanics that produce and package the staged tree.

The Exasol engine sets `TZ` from the session timezone for every UDF (via `NSEXEC_ENV_TZ` → `TZ`), commonly as an IANA name such as `Europe/Berlin`. The staged tree must carry the IANA zoneinfo database so `chrono::Local`/`time` resolve named zones instead of silently falling back to UTC; the runtime never reads `TZ` itself.

The SLC is distributed as a flattened root-filesystem tarball that Exasol extracts after BucketFS upload, with the executable at `/exaudf/exaudfclient`. For DNS to work inside the UDF sandbox, the tarball must present `/etc/hosts` and `/etc/resolv.conf` as symlinks into `/conf/`, which the database populates at runtime. These symlinks cannot be baked as live symlinks in the image layers (`COPY` dereferences a dangling symlink into a 0-byte file; `RUN ln -sf` hits Docker's build-time bind-mount of those two paths), so they are created in a staging directory and tarred inside the Docker build itself.

## Scenarios

### Scenario: docker build produces the SLC artifact tarball

* *GIVEN* the workspace with a single `Dockerfile` at the repository root and no Alpine-qualified or Debian-qualified Dockerfile variant beside it
* *WHEN* `docker build --target artifact --output type=local,dest=<dir> .` is run
* *THEN* the build MUST complete successfully and write `lc-rs.tar.gz` into `<dir>`
* *AND* the tarball MUST contain `exaudf/exaudfclient` as an executable regular file

### Scenario: Builder toolchain and glibc runtime

* *GIVEN* the Dockerfile builder stage `FROM rust:1.94-trixie`
* *WHEN* the SLC is built
* *THEN* the builder MUST install `protobuf-compiler` and `pkg-config` but NOT `libzmq3-dev` and NOT `libbz2-dev`, so zmq is statically linked via `zeromq-src`; the builder installs no bzip2 development package because `exaudfclient` links no bzip2 at all
* *AND* the builder MUST derive the Debian multiarch triplet and the built binary's own `PT_INTERP` loader path at build time rather than hardcoding `x86_64-linux-gnu`, and MUST record both for the staging stage, which carries neither `binutils` nor `dpkg-architecture` of its own
* *AND* an empty derived triplet or an empty derived loader path MUST fail the build with an error naming the command that produced nothing, rather than a cryptic downstream `cp` failure
* *AND* the builder image tag MUST match the channel pinned in `rust-toolchain.toml` (`1.94`); the spec MUST NOT name a stale builder tag that no longer matches the toolchain pin

### Scenario: SLC builds natively for the host architecture

* *GIVEN* a build host of architecture `x86_64` or `aarch64` with Docker
* *WHEN* `docker build --target artifact` runs against the root `Dockerfile` natively, without QEMU emulation or cross-compilation
* *THEN* the produced `exaudf/exaudfclient` MUST be an ELF binary of the build host's architecture
* *AND* the staged tree MUST resolve that binary's own `PT_INTERP` path to a real loader file through the staged usr-merge symlinks, so the UDF sandbox finds its interpreter instead of every UDF dying as a bare `22002 VM crashed`
* *AND* the resulting SLC MUST be registrable and execute UDFs on an Exasol host of that architecture, with no change to the `language_definitions.json` contract

### Scenario: Staged tree reproduces the donor's usr-merge layout

* *GIVEN* the `debian:trixie-slim` staging stage, whose own root has `/lib`, `/bin` and `/sbin` as symlinks into `/usr`, plus `/lib64` on x86_64 but not on aarch64
* *WHEN* the `/slc` tree is staged
* *THEN* each of those four top-level paths that is a symlink in the donor MUST be reproduced in `/slc` as a symlink carrying the donor's own target, read from the donor rather than hardcoded per architecture
* *AND* every staged regular file MUST live under `/slc/usr`, so a reproduced symlink resolves to staged content instead of being shadowed by a real directory of the same name
* *AND* the staged tree MUST NOT invent a `/lib64` entry on an architecture whose donor has none

### Scenario: Runtime stage is slim and self-sufficient

* *GIVEN* the `debian:trixie-slim` staging stage
* *WHEN* the `/slc` tree is staged
* *THEN* the stage MUST `apt-get install` only `ca-certificates` and `tzdata`; every other staged library MUST already be present in the base image
* *AND* it MUST set `ENV LANG=C.UTF-8` and MUST also stage `/usr/lib/locale/C.utf8` into the tree, because the image-level `ENV` does not survive tarball extraction — the staged locale data is what makes `C.UTF-8` resolvable inside the UDF sandbox — and no `locale-gen` MUST be run and no locale package installed
* *AND* the staged tree MUST NOT contain a Rust toolchain, a vendored Cargo registry, a shell, a package manager or coreutils

### Scenario: Language definitions file is present and well-formed

* *GIVEN* the SLC tarball
* *WHEN* `build_info/language_definitions.json` is read from it
* *THEN* it MUST declare `schema_version` `2`
* *AND* it MUST contain one language definition with protocol `localzmq+protobuf`, alias `RUST`, parameter `lang=rust`, and `udf_client_path.executable` equal to `/exaudf/exaudfclient`

### Scenario: Staged tree passes an in-build chroot self-test

* *GIVEN* the staged `/slc` tree, before it is tarred
* *WHEN* the build runs `chroot /slc /exaudf/exaudfclient` with no further arguments
* *THEN* the client MUST report a wrong-argument-count error and exit non-zero, and the build MUST fail if it does not
* *AND* a `/slc` tree missing the loader or a usr-merge symlink MUST therefore fail the build as a `chroot` failure, instead of passing the build and surfacing downstream as a bare `22002 VM crashed`
* *AND* on `aarch64`, where CI has no live Exasol database, this self-test together with the tarball's structural assertions MUST remain the structural coverage and MUST NOT be removed

### Scenario: Debian-staged SLC passes the db-roundtrip integration suite

* *GIVEN* the SLC tarball built from the root `Dockerfile` and a running `exasol/docker-db` container from the supported version matrix
* *WHEN* the db-roundtrip integration harness registers the SLC, uploads the UDF artifacts and runs every roundtrip scenario
* *THEN* the scalar, set/EMITS, statically-linked-dependency, UDF-error, single-call, name-resolution and session-timezone scenarios MUST all pass against the Debian-staged SLC
* *AND* replacing the runtime base MUST require no change to the `language_definitions.json` contract and no change to how `SCRIPT_LANGUAGES` names the executable

### Scenario: Staged tarball carries only the curated runtime surface

* *GIVEN* the SLC tarball
* *WHEN* its entries are enumerated
* *THEN* it MUST NOT contain a shell, a package manager or a coreutils binary — no `bin/sh`, no `usr/bin/apt`, no `usr/bin/dpkg`
* *AND* the compressed tarball MUST stay under a committed size ceiling, so accidentally staging the donor's whole root filesystem or its full library surface fails the pipeline instead of shipping
* *AND* the check that enforces the ceiling MUST also report the measured compressed and uncompressed sizes, so any deliberate growth is a visible, reviewed number rather than a silent drift

### Scenario: SLC tarball ships the /conf resolver symlinks

* *GIVEN* the SLC distribution tarball produced from the root `Dockerfile` by the Docker build alone, without any host-side post-processing step
* *WHEN* the entries for `etc/hosts` and `etc/resolv.conf` are inspected
* *THEN* `etc/hosts` MUST be a symbolic-link entry pointing to `/conf/hosts`
* *AND* `etc/resolv.conf` MUST be a symbolic-link entry pointing to `/conf/resolv.conf`
* *AND* producing the tarball MUST NOT require any interpreter or tool outside the Docker build environment (no host `python3`)
* *AND* the tarball MUST be produced with GNU `tar --hard-dereference`, so no shipped path depends on BucketFS extraction recreating a hard link

### Scenario: Runtime image bundles the IANA zoneinfo database

* *GIVEN* the SLC tarball and that the database always sends the session timezone as `TZ` for every UDF
* *WHEN* the tarball is inspected for the zoneinfo database
* *THEN* `usr/share/zoneinfo/Europe/Berlin` MUST be present as a readable, non-empty regular file, not a link whose target could be lost in extraction
* *AND* the fix MUST remain packaging only (an `apt-get install` of `tzdata`), since `chrono`/`time` consult the zoneinfo database implicitly and the runtime MUST NOT read `TZ` itself
