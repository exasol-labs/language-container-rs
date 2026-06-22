# Feature: workspace-bootstrap

Bootstrap the `lc-rs` Cargo workspace from an empty repository: create the workspace manifest, pin the Rust toolchain, scaffold all crate stubs, and produce a fully building `exa-proto` crate with a vendored proto and prost-build code generation.

## Background

This delta tightens the centralized `[workspace.dependencies]` table: the `indexmap` entry is declared but referenced by no member crate, so it MUST be dropped to keep the dependency graph and `Cargo.lock` lean.

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Workspace Cargo.toml is well-formed

* *GIVEN* an empty repository with no Cargo files
* *WHEN* `Cargo.toml` is created at the workspace root
* *THEN* it MUST declare `[workspace]` with `resolver = "2"` and list all crate members under `members`
* *AND* it MUST include a `[workspace.dependencies]` table centralizing all shared dependencies (`zmq`, `prost`, `libloading`, `arrow`, `syn`, `quote`, `thiserror`, `tracing`)
* *AND* the `[workspace.dependencies]` table MUST NOT declare `indexmap`, because no member crate references it (a declared-but-unused workspace dependency only bloats the dependency graph and `Cargo.lock`)
* *AND* it MUST include `[patch.crates-io]` pointing `exarrow-rs` to the local path
<!-- /DELTA:CHANGED -->
