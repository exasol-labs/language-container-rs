# Feature: os-license-notices

Generates and ships an OS-layer attribution manifest (`THIRD-PARTY-OS-LICENSES.md`) into the SLC tarball to satisfy the attribution and source-offer obligations of the Debian runtime libraries staged into the container.

## Background

The distributed SLC artifact `lc-rs.tar.gz` (built from the root `Dockerfile --target artifact`) bundles no distribution rootfs at all: it carries a curated set of Debian 13 (trixie) runtime libraries copied with `cp -L` out of the `debian:trixie-slim` staging stage — glibc and its NSS/resolver modules, `libgcc_s`/`libstdc++`, `ca-certificates`, `tzdata`, OpenSSL 3 with its provider modules, and the `zlib`/`bzip2`/`zstd` compression libraries. The cargo-generated `THIRD-PARTY-LICENSES.md` covers only the Rust dependency graph of `exaudfclient`; those staged libraries carry separate attribution and source-offer obligations under licenses such as LGPL-2.1, GPL-3.0 with the GCC Runtime Library Exception 3.1, MPL-2.0, Apache-2.0, MIT, Zlib, BSD-3-Clause and bzip2-1.0.6.

Because no package manager, shell or coreutils binary is staged, the notice set is the staged library list rather than an installed-package database, and the obligations narrow accordingly: dropping the previous Alpine rootfs removed every GPL-2.0-only and BSD-2-Clause component from the shipped artifact, and the written source offer needs only one upstream — Debian.

This is an attribution and source-offer compliance concern only. All bundled components are unmodified upstream binaries shipped as separate files (glibc is LGPL-2.1; libstdc++/libgcc carry the GCC Runtime Library Exception), so nothing propagates to `exaudfclient` or to user UDFs.

The OS-layer notice bundle is **generated**, not hand-maintained: boilerplate lives in `dist/` and a generator renders the license texts from `cargo about`'s embedded SPDX data, so no verbatim license text is committed to the repository.

## Scenarios

### Scenario: OS-layer license generator boilerplate is committed under dist/

* *GIVEN* the distributed SLC tarball (`lc-rs.tar.gz`) bundles the Debian runtime libraries staged from `debian:trixie-slim`, none of which are covered by the cargo-generated `THIRD-PARTY-LICENSES.md`
* *WHEN* the repository is inspected for the OS-layer attribution tooling
* *THEN* a `dist/` directory MUST contain the committed generator boilerplate: a `cargo about` config (`about-os.toml`) whose accepted list enumerates the staged libraries' SPDX licenses, a synthetic dependency-free crate (`os-attribution/Cargo.toml`) whose `license` expression names those same licenses, a Handlebars template (`os-licenses.hbs`) carrying the staged-library → SPDX table and a three-year written source offer, and the generator script (`generate-licenses.sh`)
* *AND* the `accepted` list and the synthetic crate's `license` expression MUST name the same license set, so neither can drift from the other
* *AND* the generated manifest `dist/THIRD-PARTY-OS-LICENSES.md` MUST be git-ignored (it embeds full license texts), so no verbatim license text is committed to the repository

### Scenario: The generator renders a complete OS-license manifest via cargo-about

* *GIVEN* the committed `dist/` boilerplate and `cargo about` (0.9.0) available on PATH
* *WHEN* `dist/generate-licenses.sh` is run
* *THEN* it MUST produce `dist/THIRD-PARTY-OS-LICENSES.md` containing a staged-library → SPDX table that covers glibc with its NSS/resolver modules and loader, `libgcc_s`/`libstdc++`, `ca-certificates`, `tzdata`, OpenSSL 3 with its provider modules, `zlib`, `zstd` and `bzip2`
* *AND* it MUST reproduce the canonical text of every license the staged set carries — at minimum LGPL-2.1, the GPL-3.0 base text, MPL-2.0, Apache-2.0, MIT, Zlib, BSD-3-Clause and bzip2-1.0.6 — rendered from SPDX data by `cargo about` where it can, and appended from the SPDX license-list-data where `cargo about` emits nothing (as it already does for a `WITH` clause)
* *AND* it MUST append the GCC Runtime Library Exception 3.1 text, so the `libstdc++`/`libgcc_s` attribution is complete
* *AND* the manifest MUST carry a three-year written source offer naming only Debian sources (`snapshot.debian.org` and `sources.debian.org`), and MUST NOT reference an Alpine `aports` tree or an apk package database that the artifact no longer ships

### Scenario: The Dockerfile ships the generated OS-license manifest into the tarball

* *GIVEN* the root `Dockerfile` already stages `LICENSE` and `THIRD-PARTY-LICENSES.md` into `/exaudf`, and the manifest is generated into `dist/` before the build (by `scripts/install.sh`, `scripts/ci-it-local.sh` and the CI `build-slc` job)
* *WHEN* the SLC tree is staged
* *THEN* the `Dockerfile` MUST also stage `dist/THIRD-PARTY-OS-LICENSES.md` into `/exaudf` alongside the existing `LICENSE` and `THIRD-PARTY-LICENSES.md`
* *AND* the staging MUST happen before the tree is tarred, so the manifest is carried into `lc-rs.tar.gz`

### Scenario: Distributed tarball carries the OS-layer notice at /exaudf

* *GIVEN* the SLC tarball produced from the root `Dockerfile --target artifact` (the same artifact `scripts/install.sh` uploads to BucketFS), built after the generator has run
* *WHEN* the `exaudf/` entries of the extracted tarball are inspected
* *THEN* `exaudf/THIRD-PARTY-OS-LICENSES.md` MUST be present as a regular file containing the staged-library → SPDX table, the Debian written source offer, and the reproduced license texts including the GCC Runtime Library Exception and the bzip2 license
* *AND* the existing `exaudf/LICENSE` and `exaudf/THIRD-PARTY-LICENSES.md` files MUST still be present, and the db-roundtrip integration suite MUST still pass against the SLC built with the regenerated notice bundle
