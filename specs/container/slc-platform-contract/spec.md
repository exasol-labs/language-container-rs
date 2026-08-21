# Feature: slc-platform-contract

Publishes the SLC's staged glibc floor and documented dynamic-library surface as a machine-checkable contract, so an author's `.so` and the container agree on what may link dynamically at run time.

## Background

The staged tree is the UDF's entire root filesystem at run time, so it — not the Exasol host — decides what a UDF `.so` may link against dynamically. The SLC therefore provides a fixed, documented library surface: the glibc runtime (including the `libpthread`/`libdl`/`librt` compatibility stubs), `libgcc_s`/`libstdc++`, the NSS and resolver modules glibc `dlopen`s, OpenSSL 3 with its `ossl-modules`/`engines-3` providers, and the `zlib`/`bzip2`/`zstd` compression libraries. Anything else a UDF links dynamically must be vendored into the `.so`. The glibc constraint is per-symbol, not per-distro: raising the bundled glibc shrinks the trap of an author host stamping a too-new `GLIBC_x.y` version reference into an artifact, but it does not close it — so the floor is recorded as a single machine-readable value that both the container build and `cargo exasol-udf validate` read.

This contract has two enforcement points: the container build verifies the staged tree actually provides the documented surface and that the shipped client stays within the recorded floor; `cargo exasol-udf validate` (in `tools/cargo-exaudf-validate`) checks an author's own artifact against the same floor and surface before it ever reaches the container.

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
