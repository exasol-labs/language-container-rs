# Feature: os-license-notices

Generates and ships an OS-layer attribution manifest (`THIRD-PARTY-OS-LICENSES.md`) into the SLC tarball to satisfy the attribution and source-offer obligations of the bundled Alpine apk packages and glibc/GCC runtime libraries.

## Background

The distributed SLC artifact `lc-rs.tar.gz` (built from `Dockerfile.alpine --target artifact`) bundles the `alpine:3` rootfs plus glibc/libstdc++ runtime libraries copied via `cp -L` from the `rust:1.94-bookworm` (Debian 12) builder stage. The cargo-generated `THIRD-PARTY-LICENSES.md` covers only the Rust dependency graph of `exaudfclient`; the bundled OS and runtime third-party components carry separate attribution and source-offer obligations under licenses such as GPL-2.0, LGPL-2.1, MPL-2.0, Apache-2.0, MIT, BSD-2-Clause, Zlib, and the GCC Runtime Library Exception 3.1.

This is an attribution and source-offer compliance concern only. All bundled components are unmodified upstream binaries shipped as separate files (glibc is LGPL-2.1; libstdc++/libgcc carry the GCC Runtime Library Exception), so nothing propagates to `exaudfclient` or to user UDFs.

The OS-layer notice bundle is **generated**, not hand-maintained: boilerplate lives in `dist/` and a generator renders the license texts from `cargo about`'s embedded SPDX data, so no verbatim license text is committed to the repository.

## Scenarios

### Scenario: OS-layer license generator boilerplate is committed under dist/

* *GIVEN* the distributed SLC tarball (`lc-rs.tar.gz`, produced from `Dockerfile.alpine`) bundles `alpine:3` apk packages and the glibc/GCC runtime libs copied from the `rust:1.94-bookworm` builder, none of which are covered by the cargo-generated `THIRD-PARTY-LICENSES.md`
* *WHEN* the repository is inspected for the OS-layer attribution tooling
* *THEN* a `dist/` directory MUST contain the committed generator boilerplate: a `cargo about` config (`about-os.toml`) whose accepted list enumerates the bundled OS/runtime SPDX licenses, a synthetic dependency-free crate (`os-attribution/Cargo.toml`) whose `license` expression names those same licenses, a Handlebars template (`os-licenses.hbs`) carrying the apk package → SPDX table, the copied-glibc/GCC-libs section, and a three-year written source offer, and a generator script (`generate-os-licenses.sh`)
* *AND* the generated manifest `dist/THIRD-PARTY-OS-LICENSES.md` MUST be git-ignored (it embeds full license texts), so no verbatim license text is committed to the repository

### Scenario: The generator renders a complete OS-license manifest via cargo-about

* *GIVEN* the committed `dist/` boilerplate and `cargo about` (0.9.0) available on PATH
* *WHEN* `dist/generate-os-licenses.sh` is run
* *THEN* it MUST produce `dist/THIRD-PARTY-OS-LICENSES.md` containing the apk package → SPDX table, the copied glibc/GCC runtime-libs section with file paths, and the three-year written source offer pointing recipients at Alpine aports (for the apk package versions) and Debian source / snapshot.debian.org (for glibc and the gcc-12 runtime)
* *AND* it MUST reproduce the canonical text of every bundled license — at minimum GPL-2.0-only, LGPL-2.1, MPL-2.0, Apache-2.0, MIT, BSD-2-Clause, Zlib, and the GPL-3.0 base text — rendered by `cargo about` from its embedded SPDX data
* *AND* it MUST append the GCC Runtime Library Exception 3.1 text (which `cargo about` does not emit for a `WITH` clause), so the libstdc++/libgcc attribution is complete

### Scenario: Dockerfile.alpine ships the generated OS-license manifest into the tarball

* *GIVEN* `Dockerfile.alpine` already COPYs `LICENSE` and `THIRD-PARTY-LICENSES.md` into `/exaudf` in its runtime stage, and the manifest is generated into `dist/` before the build (by `scripts/install.sh`, `scripts/ci-it-local.sh`, and the CI `build-slc` job)
* *WHEN* the runtime stage is built
* *THEN* `Dockerfile.alpine` MUST also COPY `dist/THIRD-PARTY-OS-LICENSES.md` into `/exaudf` alongside the existing `LICENSE` and `THIRD-PARTY-LICENSES.md`
* *AND* the COPY MUST be in the `runtime` stage (before the `packager` stage tars the rootfs) so the manifest is carried into `lc-rs.tar.gz`

### Scenario: Distributed tarball carries the OS-layer notice at /exaudf

* *GIVEN* the SLC tarball produced from `Dockerfile.alpine --target artifact` (the same artifact `scripts/install.sh` uploads to BucketFS), built after the generator has run
* *WHEN* the `exaudf/` entries of the extracted tarball are inspected
* *THEN* `exaudf/THIRD-PARTY-OS-LICENSES.md` MUST be present as a regular file containing the apk → SPDX table, the glibc/GCC section, the written source offer, and the reproduced license texts including the GCC Runtime Library Exception
* *AND* the existing `exaudf/LICENSE` and `exaudf/THIRD-PARTY-LICENSES.md` files MUST still be present, and the existing db-roundtrip integration suite MUST still pass against the image built with the added notice bundle
