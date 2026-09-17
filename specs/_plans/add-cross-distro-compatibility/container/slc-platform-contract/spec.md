# Feature: slc-platform-contract

Publishes the SLC's staged glibc floor, builder identity, dynamic-library surface and sandbox directory skeleton as a machine-checkable contract, so an author's `.so`, the author's host and the container all agree on what runs at run time.

## Background

The staged tree is the UDF's entire root filesystem at run time, so it — not the Exasol host — decides what a UDF `.so` may link against dynamically. The SLC therefore provides a fixed, documented library surface: the glibc runtime (including the `libpthread`/`libdl`/`librt` compatibility stubs), `libgcc_s`/`libstdc++`, the NSS and resolver modules glibc `dlopen`s, OpenSSL 3 with its `ossl-modules`/`engines-3` providers, and the `zlib`/`bzip2`/`zstd` compression libraries. Anything else a UDF links dynamically must be vendored into the `.so`. The glibc constraint is per-symbol, not per-distro: raising the bundled glibc shrinks the trap of an author host stamping a too-new `GLIBC_x.y` version reference into an artifact, but it does not close it — so the floor is recorded as a single machine-readable value that both the container build and `cargo exasol-udf validate` read.

The vtable fingerprint the host runtime compares at load time is `SDK_VERSION:RUSTC_IDENTITY`, and the runtime's own copy is baked from the rustc that compiled the shipped `exaudfclient`. An artifact therefore loads only when its rustc identity equals the SLC builder's, which no author host satisfies by accident and no distribution-packaged Rust toolchain satisfies at all. The builder image reference, the rustc identity it provides and the toolchain version it installs always change together, so they are recorded as one committed builder record that the container build verifies against its own builder stage. The SDK is the single owner of how a rustc version string becomes a fingerprint part, so the record holds that part verbatim rather than a string re-derived from `rustc --version` by anything else. `cargo exasol-udf build --container` reads the image reference from that record, and `cargo exasol-udf validate` reads the rustc identity from it.

This feature covers what the SLC image guarantees to an author. What the database host must provide before that image can start a UDF at all is a separate contract, specified in `container/host-runtime-compatibility`, because it is decided by the host kernel rather than by anything the image carries.

The Exasol UDF sandbox prepares mount points inside the SLC root before the client starts. It creates a missing directory when that root is writable, and reports `cannot create directories: Read-only file system` when it is not. A deployment that extracts the tarball into writable BucketFS storage therefore hides a missing directory, and a deployment that mounts the SLC read-only turns the same gap into `22002 VM crashed` on every call. Exasol Personal mounts each installed SLC read-only, by the same code path for a catalog SLC and for a custom one, so the mount mode is not what separates a working SLC from a failing one. What separates them is the skeleton: every official SLC is exported from a full distribution image and creates `conf` and `buckets` on top of it, while a tree staged from scratch carries neither. The skeleton is therefore recorded as one committed list, mirroring the top-level set an official SLC provides, so the container build stages exactly that set and the tarball check asserts exactly that set.

This contract has three enforcement points: the container build verifies the staged tree provides the documented surface, the recorded skeleton and the recorded builder identity, and that the shipped client stays within the recorded floor; `cargo exasol-udf validate` (in `tools/cargo-exaudf-validate`) checks an author's own artifact against the floor, the builder rustc identity and the surface before it ever reaches the container; and `cargo exasol-udf build --container` (in `tools/cargo-exaudf`) builds against the recorded builder image so an artifact satisfies the floor and the identity by construction.

## Scenarios

### Scenario: Staged tree provides the documented UDF library surface

* *GIVEN* the staged `/slc` tree
* *WHEN* its ELF contents are enumerated
* *THEN* it MUST stage the glibc runtime (`libc.so.6`, `libm.so.6` and the `libpthread.so.0`/`libdl.so.2`/`librt.so.1` compatibility stubs), `libgcc_s.so.1`, `libstdc++.so.6`, the loader at the client's `PT_INTERP` path, the dlopen-only `libnss_files.so.2`, `libnss_dns.so.2` and `libresolv.so.2`, `libssl.so.3`, `libcrypto.so.3`, the OpenSSL `ossl-modules` and `engines-3` providers, `libz.so.1`, `libbz2.so.1` and `libzstd.so.1`
* *AND* every `DT_NEEDED` entry of every staged ELF file MUST resolve to a file inside the staged tree, so a UDF that `dlopen`s any staged library does not fail on a missing transitive dependency
* *AND* the staged `/etc/nsswitch.conf` MUST name only services whose NSS modules are staged, and the OpenSSL default trust path MUST resolve inside the tree to the staged `ca-certificates` bundle
* *AND* the staged surface MUST NOT be widened to the donor's full library set; anything a UDF needs beyond this surface MUST be vendored into the UDF `.so`

### Scenario: Staged glibc defines the documented author floor

* *GIVEN* the single committed machine-readable glibc floor that `cargo exasol-udf validate` and the user documentation both read
* *WHEN* the staged `libc.so.6` is inspected
* *THEN* the recorded floor MUST equal the highest `GLIBC_x.y` version the staged `libc.so.6` defines, so the published floor cannot drift from the shipped container
* *AND* the shipped `exaudfclient`'s own highest referenced `GLIBC_x.y` version MUST NOT exceed that floor
* *AND* a mismatch MUST fail the build pipeline, rather than being discovered by an author whose `.so` fails at `dlopen` inside the container

<!-- DELTA:NEW -->
### Scenario: Extracted tree provides the sandbox directory skeleton

* *GIVEN* the committed sandbox skeleton record, which names every directory the Exasol UDF sandbox needs inside the SLC root that the staged tree does not already provide
* *WHEN* the distributed tarball's directory entries are enumerated
* *THEN* the tarball MUST carry a directory entry for every path the record names, so the sandbox creates none of them and a read-only SLC root is enough to start a UDF
* *AND* each such entry MUST be an empty directory, so the skeleton adds mount points without adding content to the published library surface or the size ceiling
* *AND* the record MUST name `proc`, `var/tmp` and `buckets`, the three paths a read-only deployment was observed to fail on in that order, `conf`, which every official flavor creates and which the staged `etc/hosts` and `etc/resolv.conf` symlinks already point into, and every other top-level directory an official SLC root provides that the staged tree lacks, so the set follows a published layout rather than growing one failure at a time
* *AND* a tarball missing any recorded path MUST fail the build pipeline, rather than surfacing on a read-only deployment as a bare `22002 VM crashed` with the cause visible only through `SCRIPT_OUTPUT_ADDRESS`
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: SLC publishes the builder identity its loader enforces

* *GIVEN* the single committed builder record holding the builder image reference on its first line, the rustc identity that image provides on its second, and the exact toolchain version that image provides on its third
* *WHEN* the container is built
* *THEN* the recorded identity MUST be the fingerprint's rustc-identity part verbatim, taken from the fingerprint string the SDK baked into the compiled `exaudfclient`, so the SDK stays the only component that decides how a rustc version string becomes a fingerprint part
* *AND* the build MUST compare that part against the record and MUST fail on a mismatch, so the record cannot drift from the toolchain that compiled the shipped `exaudfclient`
* *AND* the recorded image reference MUST equal the reference the builder stage is actually built from, verified against the build definition, so the record and the image the container build runs cannot diverge
* *AND* the recorded image reference MUST pin the exact toolchain patch version the third line names, and that third line MUST be the toolchain version an author installs to reach the `host` build path, so an upstream patch release behind a moving tag cannot change the recorded identity and no reader has to recover a toolchain version from an image tag or from the sanitized fingerprint string
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: Published platform support names one build path per author host

* *GIVEN* the single committed list of supported author-host platforms, each line carrying exactly five fields: the platform family, the platform version, the container image reference the distribution harness verifies that row with, the glibc version that image provides, and the build path, which is `host` or `container`
* *WHEN* the user documentation's author-host support statement is checked against that list
* *THEN* the statement MUST name every row the list names, by its family and its version, with the same build path, so a published compatibility claim cannot drift from the set that is verified
* *AND* a build path of `host` MUST mean both that the recorded glibc is at or below the recorded floor and that the author installed, through rustup, the exact toolchain version the builder record's third line names, because a distribution-packaged Rust toolchain never produces the recorded rustc identity whatever its version number
* *AND* every row that fails either of those two conditions MUST carry the build path `container`, and a row that names no image reference MUST carry a placeholder in that field, MUST carry the build path `container`, and MUST contribute no distribution-harness leg, because the harness cannot verify a host build path it cannot run
* *AND* the list MUST cover each distribution family Exasol's own published requirements name for a database host or a client host, MUST cover macOS, and MUST use the family and version pair as its row key, so the claim spans every platform an author builds on, a family whose claim covers two versions occupies two rows, and no consumer addresses a row by a family that cannot separate two versions carrying different images
<!-- /DELTA:NEW -->
