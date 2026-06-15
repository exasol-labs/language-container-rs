# Feature: workspace-bootstrap

Bootstrap the `slc-rs` Cargo workspace from an empty repository: create the workspace manifest, pin the Rust toolchain, scaffold all seven crate stubs, and produce a fully building `exa-proto` crate with a vendored proto and prost-build code generation.

## Background

The repository starts as an empty git repo with only `CLAUDE.md` and `specs/`. No `Cargo.toml`, no `crates/` directory, and no toolchain pin exist yet. The workspace MUST compile `exa-proto` as the first concrete deliverable; the remaining six crates are stubs that compile to empty libraries or binaries until later phases fill them in.

The `exarrow-rs` crate lives at `/home/talos/code/exarrow-rs` (not on crates.io) and MUST be patched in via `[patch.crates-io]` so the shared `arrow = "58"` dependency is deduplicated across the workspace, even though connect-back is out of scope for v1.

## Scenarios

### Scenario: Workspace Cargo.toml is well-formed

* *GIVEN* an empty repository with no Cargo files
* *WHEN* `Cargo.toml` is created at the workspace root
* *THEN* it MUST declare `[workspace]` with `resolver = "2"` and list all seven crate members under `members`
* *AND* it MUST include a `[workspace.dependencies]` table centralizing all shared dependencies (`zmq`, `prost`, `libloading`, `arrow`, `syn`, `quote`, `thiserror`, `tracing`)
* *AND* it MUST include `[patch.crates-io]` pointing `exarrow-rs` to the local path `/home/talos/code/exarrow-rs`

### Scenario: All seven crate stubs exist and compile

* *GIVEN* the workspace `Cargo.toml` lists seven members
* *WHEN* `cargo build --workspace` is invoked
* *THEN* each of the seven crates MUST compile without errors
* *AND* each library crate stub MUST contain an `src/lib.rs` that is valid Rust
* *AND* each binary crate stub (`exaudfclient`, `cargo-exaudf`) MUST contain an `src/main.rs` with a minimal `fn main()`

### Scenario: exa-proto vendors zmqcontainer.proto

* *GIVEN* no proto file exists in the repository
* *WHEN* `crates/exa-proto/proto/zmqcontainer.proto` is created by downloading from the GitHub raw URL
* *THEN* the file MUST be byte-identical to the canonical proto at `https://github.com/exasol/script-languages/raw/master/exaudfclient/base/exaudflib/zmqcontainer.proto`
* *AND* `PROTO_SOURCES.md` MUST record the source URL, the git commit SHA, and the fetch date

### Scenario: exa-proto build.rs generates Rust bindings

* *GIVEN* `crates/exa-proto/proto/zmqcontainer.proto` is present
* *AND* `crates/exa-proto/build.rs` uses `prost_build::compile_protos` with no extra `Config` options
* *WHEN* `cargo build -p exa-proto` is invoked
* *THEN* `prost-build` MUST generate Rust source into `OUT_DIR` without errors
* *AND* `build.rs` MUST emit `cargo:rerun-if-changed=proto/zmqcontainer.proto`
* *AND* the generated code MUST include the prost-annotated structs for `exascript_request` and `exascript_response`

### Scenario: exa-proto lib.rs re-exports generated types

* *GIVEN* `build.rs` has placed generated code in `OUT_DIR`
* *WHEN* `src/lib.rs` uses `include!` to pull in the generated file and re-exports the top-level items
* *THEN* downstream crates MUST be able to reference `exa_proto::ExascriptRequest` without additional path qualifications
* *AND* `cargo build -p exa-proto` MUST exit with code 0
