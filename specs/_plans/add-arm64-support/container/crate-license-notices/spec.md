# Feature: crate-license-notices

Generates and ships the Rust dependency-graph attribution manifest (`THIRD-PARTY-LICENSES.md`) into the SLC tarball, covering every architecture the container is distributed for so no linked-in dependency goes unattributed.

## Background

The distributed SLC bundles the `exaudfclient` binary, whose Rust dependency graph carries per-crate license obligations distinct from the OS-layer packages covered by `os-license-notices`. `dist/generate-licenses.sh` runs `cargo about generate` over `crates/exaudfclient/Cargo.toml` with `about.toml` and `about.hbs`, producing `dist/THIRD-PARTY-LICENSES.md`, which `Dockerfile.alpine` COPYs into `/exaudf` and carries into `lc-rs.tar.gz`.

`about.toml`'s `targets` array selects which target triples `cargo about` evaluates. cargo-about applies these **as a union**: a dependency reached only through a `cfg(...)` gate is attributed if it matches **any** listed target, and nothing is dropped from the other targets' sets. Omitting an architecture the SLC actually ships silently drops that architecture's `cfg`-gated dependencies from the manifest — an attribution/compliance defect — whereas listing extra targets only over-attributes, which is safe.

This is an attribution and source-offer compliance concern only; it does not change what `exaudfclient` or user UDFs do at runtime.

## Scenarios

### Scenario: License bundle covers every shipped architecture

* *GIVEN* the SLC is distributed for both `x86_64` and `aarch64` hosts
* *WHEN* `dist/generate-licenses.sh` renders `THIRD-PARTY-LICENSES.md`
* *THEN* `about.toml`'s `targets` array MUST enumerate every architecture the SLC is distributed for, so cargo-about's union includes each architecture's `cfg`-gated dependencies
* *AND* the manifest MUST NOT lose any dependency already attributed for `x86_64` when `aarch64` is added, since the evaluation is a union and not an intersection

### Scenario: Target set reflects the shipped glibc binary

* *GIVEN* `exaudfclient` is compiled by the `rust:1.94-bookworm` builder with no `--target`, so the shipped binary is glibc (`<arch>-unknown-linux-gnu`), not musl
* *WHEN* the `targets` array is chosen
* *THEN* it MUST include the glibc (`-unknown-linux-gnu`) triple for every shipped architecture so `gnu`-gated dependencies are attributed, correcting the prior musl-only pin that under-reported the shipped binary
* *AND* it MAY additionally list the corresponding `-unknown-linux-musl` triples, since the union makes the extra entries harmless
* *AND* a comment in `about.toml` MUST record why the shipped binary is glibc and why both libc/arch triples are listed

### Scenario: Generated manifest ships in the tarball for each architecture

* *GIVEN* an SLC tarball built from `Dockerfile.alpine --target artifact` for a given architecture, after the generator has run
* *WHEN* the `exaudf/` entries of the extracted tarball are inspected
* *THEN* `exaudf/THIRD-PARTY-LICENSES.md` MUST be present as a regular file carrying the union attribution for the shipped architectures
* *AND* the existing `exaudf/LICENSE` and `exaudf/THIRD-PARTY-OS-LICENSES.md` files MUST still be present
