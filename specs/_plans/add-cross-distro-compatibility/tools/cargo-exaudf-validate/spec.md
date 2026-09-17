# Feature: cargo-exaudf-validate

Provides `cargo exasol-udf validate`, which checks a built `.so`'s ABI compatibility with the current SDK and its dlopen-time platform fit against the SLC's glibc floor and library surface — so an author learns about a load failure on their own machine, not inside the container.

## Background

`validate` is a subcommand of the `cargo exasol-udf` CLI (`tools/cargo-exaudf`). ABI compatibility is judged against the same `EXA_UDF_ABI_VERSION` and `EXA_SDK_FINGERPRINT` constants baked into `exasol-udf-sdk`.

The ABI fingerprint covers the SDK version and the compiling rustc, not the platform, so it cannot catch the two remaining ways a well-formed `.so` still fails at `dlopen` inside the container: a glibc symbol-version reference newer than the SLC's bundled glibc, and a dynamic dependency on a library the SLC does not stage. `validate` therefore reads the artifact's ELF dynamic section once and derives all three facts from that single read — the exported entry symbols, the `DT_NEEDED` sonames, and the highest referenced `GLIBC_x.y` version. The SLC's library surface and its glibc floor are container facts, specified in `container/slc-platform-contract`, not CLI facts: the floor is a single committed value the container build verifies against the shipped `libc.so.6`, so the number the CLI reports can never drift from what ships.

The CLI's own `EXA_SDK_FINGERPRINT` is baked when the author installs the CLI, so it carries the author's rustc identity, not the SLC's. Comparing an artifact against it therefore passes on every host and catches no toolchain skew against the container. The SLC's builder rustc identity is a third committed container fact in the same builder record, and `validate` compares the artifact against that record. The CLI release and the SLC release are published from one commit, so the recorded identity describes the SLC an author installs alongside that CLI version.

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: validate accepts a compatible .so

* *GIVEN* a `.so` built against the current `exasol-udf-sdk` exporting one or more `__exa_udf_entry_<NAME>` symbols
* *WHEN* the author runs `cargo exasol-udf validate <path.so>`
* *THEN* the CLI MUST discover every exported `__exa_udf_entry_<NAME>` symbol by reading the artifact's own ELF dynamic symbol table, without shelling out to `nm` or requiring binutils on the author's host
* *AND* for each discovered entry point it MUST dlopen the `.so` and confirm the vtable `abi_version` equals `EXA_UDF_ABI_VERSION` and the `sdk_fingerprint` matches the current SDK
* *AND* it MUST report each discovered UDF name, the artifact's highest referenced `GLIBC_x.y` version against the SLC floor, the artifact's rustc identity against the SLC's recorded builder rustc identity, and its dynamic dependencies, then exit zero
<!-- /DELTA:CHANGED -->

### Scenario: validate rejects an ABI or fingerprint mismatch

* *GIVEN* a `.so` with a `__exa_udf_entry_<NAME>` symbol whose vtable `abi_version` or `sdk_fingerprint` differs from the current SDK
* *WHEN* the author runs `cargo exasol-udf validate <path.so>`
* *THEN* the CLI MUST exit non-zero
* *AND* it MUST report which UDF name and which of `abi_version` or `sdk_fingerprint` mismatched, showing expected and actual values

### Scenario: validate rejects a .so missing any entry symbol

* *GIVEN* a shared object that exports no `__exa_udf_entry_<NAME>` symbol (including a legacy `.so` that exports only the bare `__exa_udf_entry`)
* *WHEN* the author runs `cargo exasol-udf validate <path.so>`
* *THEN* the CLI MUST exit non-zero
* *AND* it MUST report that no `__exa_udf_entry_<NAME>` entry point could be found, with a hint to rebuild against sdk >= 0.14.0
* *AND* a file that is not a parseable ELF shared object MUST be reported as such by name, rather than being reported as a shared object that merely has no entry points

### Scenario: validate reports the artifact's glibc version floor

* *GIVEN* a compiled UDF `.so` whose highest referenced glibc symbol version is at or below the glibc the SLC bundles
* *WHEN* the author runs `cargo exasol-udf validate <path.so>`
* *THEN* the CLI MUST read the artifact's glibc version-needed references from the same ELF read that discovered its entry points
* *AND* it MUST report the highest referenced `GLIBC_x.y` version alongside the SLC floor it was compared against
* *AND* it MUST exit zero, since an artifact at or below the floor resolves in the container
* *AND* an artifact that references no versioned glibc symbol at all MUST be reported as such and MUST NOT be treated as a failure

<!-- DELTA:CHANGED -->
### Scenario: validate rejects an artifact above the SLC glibc floor

* *GIVEN* a compiled UDF `.so` that references a `GLIBC_x.y` version newer than the glibc the SLC bundles
* *WHEN* the author runs `cargo exasol-udf validate <path.so>`
* *THEN* the CLI MUST exit non-zero, naming both the referenced version and the floor
* *AND* the message MUST name `cargo exasol-udf build --container` as the remedy, because that is the only build path that holds on a host whose own glibc is above the floor, and the failure would otherwise surface as a raw loader error at `dlopen` inside the container
* *AND* the floor MUST be read from the single committed value the container build verifies against the shipped `libc.so.6`, not from a number duplicated in the CLI source
<!-- /DELTA:CHANGED -->

<!-- DELTA:NEW -->
### Scenario: validate rejects an artifact built by a rustc the SLC does not run

* *GIVEN* a compiled UDF `.so` whose baked `sdk_fingerprint` carries a rustc identity different from the SLC's recorded builder rustc identity
* *WHEN* the author runs `cargo exasol-udf validate <path.so>`
* *THEN* the CLI MUST exit non-zero, naming the artifact's rustc identity and the recorded one
* *AND* the message MUST state that the container rejects this artifact at load with a fingerprint mismatch, and MUST name `cargo exasol-udf build --container` as the remedy
* *AND* the recorded identity MUST be read from the single committed builder record the container build verifies against its own builder image, so the CLI's own build-time fingerprint, which carries the author's rustc, never decides this check
* *AND* an artifact whose rustc identity equals the recorded one MUST pass this check on any host, whatever rustc that host runs
<!-- /DELTA:NEW -->

### Scenario: validate warns on dynamic dependencies outside the SLC library surface

* *GIVEN* a compiled UDF `.so` with a `DT_NEEDED` entry naming a library the SLC does not stage
* *WHEN* the author runs `cargo exasol-udf validate <path.so>`
* *THEN* the CLI MUST report every unknown dependency by soname
* *AND* the report MUST point the author at vendoring the dependency into the `.so` — a `vendored`, `bundled` or `static` feature on the offending `-sys` crate — or at a pure-Rust alternative such as `rustls-tls` in place of `native-tls`
* *AND* it MUST exit zero by default, so an artifact that happens to load today is not failed before its author can act
* *AND* it MUST treat the dynamic loader (`ld-linux-*.so.*`) and `linux-vdso.so.*` as always allowed

### Scenario: validate escalates unknown dynamic dependencies on request

* *GIVEN* a compiled UDF `.so` with a `DT_NEEDED` entry outside the SLC library surface
* *WHEN* the author runs `cargo exasol-udf validate --deny-unknown-deps <path.so>`
* *THEN* the CLI MUST exit non-zero, naming every offending soname
* *AND* the same artifact validated without the flag MUST still exit zero, so strict mode stays an opt-in for CI
* *AND* an artifact whose every `DT_NEEDED` entry lies within the SLC library surface MUST exit zero with or without the flag
