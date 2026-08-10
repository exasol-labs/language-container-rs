# Decisions: fix-glibc-cdylib-build-model

## ADR: Glibc-dynamic cdylib is the single UDF artifact model; build defaults to the host with a --target override

**ID:** glibc-dynamic-cdylib-single-artifact-model
**Plan:** fix-glibc-cdylib-build-model
**Status:** Accepted
**Supersedes:** cargo-exaudf-hides-musl-target-triple

### Context

The previous model declared every deployable UDF `.so` a fully-static musl artifact, with `cargo exasol-udf build` hiding the `x86_64-unknown-linux-musl` triple and auto-installing it via `rustup`. That path cannot produce an artifact at all: a musl target defaults `crt-static` to true, so `rustc` emits no cdylib and fails with `cannot produce cdylib ... does not support these crate types`. The glibc-dynamic cdylib is the only buildable model, and it already matches what ships — the SLC bundles the matching glibc runtime, and CI builds every fixture as `cargo build --release -p <crate>` into `target/release/lib*.so`.

### Decision

Every deployable UDF `.so` is a glibc-dynamic cdylib built by a plain host `cargo build --release`, with the artifact at `target/release/lib<crate>.so`. `cargo exasol-udf build` uses that as its default and exposes an optional `--target <triple>` that restores the per-target path `target/<triple>/release/lib<crate>.so` for a native build on a host with that target installed. The CLI performs no `rustup target add` auto-install.

### Options Considered

| Option | Verdict |
|--------|---------|
| Glibc-dynamic default plus `--target` override | ✓ Chosen — the only buildable model; matches the shipped SLC runtime and CI; a sensible default keeps a flag off the common path |
| Keep the fully-static musl `.so` model | ✗ Rejected — musl defaults `crt-static` to true, so `rustc` produces no cdylib; the musl `build` path yields no artifact |
| Drop `--target` entirely | ✗ Rejected — a cheap escape hatch for a native non-default-host build that costs nothing to keep |

### Consequences

Authors build with a plain host toolchain and no target installation step; the artifact path moves from `target/x86_64-unknown-linux-musl/release/` to `target/release/`. The musl toolchain entry, the `[target.x86_64-unknown-linux-musl]` linker stanza, and the custom target JSON become dead config and are removed. Cross-architecture builds remain the author's responsibility via `--target` on a host with that target installed.
