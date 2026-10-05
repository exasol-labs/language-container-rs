# Decisions: fix-glibc-cdylib-build-model

## ADR: Glibc-dynamic cdylib is the single UDF artifact model; build defaults to the host with a --target override

**ID:** glibc-dynamic-cdylib-single-artifact-model
**Plan:** fix-glibc-cdylib-build-model
**Status:** Accepted
**Supersedes:** cargo-exaudf-hides-musl-target-triple
**See also:** `change-slc-runtime-debian` moved the bundled glibc runtime to Debian 13; the glibc-dynamic-cdylib artifact model decided here did not change.

### Context

A musl target defaults `crt-static` to true, so `rustc` emits no cdylib and fails with `cannot produce cdylib ... does not support these crate types`. The SLC bundles the matching glibc runtime, and CI builds every fixture as `cargo build --release -p <crate>` into `target/release/lib*.so`.

### Decision

Every deployable UDF `.so` is a glibc-dynamic cdylib built by a host `cargo build --release`, at `target/release/lib<crate>.so`. `cargo exasol-udf build` uses that default and accepts an optional `--target <triple>`, which builds to `target/<triple>/release/lib<crate>.so` on a host with that target installed. The CLI does not run `rustup target add`.

### Options Considered

| Option | Verdict |
|--------|---------|
| Glibc-dynamic default plus `--target` override | ✓ Chosen |
| Fully-static musl `.so` | ✗ `rustc` produces no cdylib for musl |
| No `--target` flag | ✗ Native non-default-host builds need it, and it is cheap |

### Consequences

Authors build with a plain host toolchain. The musl toolchain entry, the `[target.x86_64-unknown-linux-musl]` linker stanza and the custom target JSON are absent. Cross-architecture builds are the author's responsibility.
