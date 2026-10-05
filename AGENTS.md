# AGENTS.md

Spec-driven development with mission in: @specs/mission.md

## Testing

- Integration and E2E tests run against a local Exasol Docker database. Start the container yourself, do not ask the user.
- Tests must fail, not skip, when Exasol is unavailable.
- Connection strings must set `validateservercertificate=0`, because the Docker image uses a self-signed certificate.

Project specifics:

- Use `exapump` for all Exasol interaction.
- Integration tests run with `cargo test -p it --features integration`. CI runs the version matrix `8.29.x / 2025.1.x / 2026.1.x`.
- `exa-udf-runtime`'s integration tests load the `test-udfs/*` cdylibs declared as its `[dev-dependencies]`. A new fixture needs a dev-dependency entry, and its crate name must equal its directory name. `scripts/build-test-udfs.sh` builds each `test-udfs/*/` as `-p <dirname>` for CI, so a mismatch fails only in CI with `reading UDF artifact .../lib<name>.so: No such file or directory`.
- `tests/emit_arrow_dlopen.rs` runs only under `--features emit-arrow-test` or `--all-features`. With any other flag set it compiles out silently.

## Code quality

- `cargo fmt --all` and `cargo clippy --all-targets` must pass with zero warnings before committing.

## Code style

- A comment states a non-obvious why: an invariant, an external-system quirk, or a spec or issue constraint. Keep it to 1 or 2 lines. Never restate the code, narrate history, or add banners. Update or delete comments when behavior changes.
- A test implementing a spec scenario carries one `/// Scenario: <title>` line per scenario, quoting the title verbatim.
- Unit tests live in `<module>_tests.rs` beside `<module>.rs`, declared as the last item of the module with `#[cfg(test)] #[path = "<module>_tests.rs"] mod tests;`. The name must match `[0-9a-zA-Z_-]+[_-]tests.rs`, or `cargo llvm-cov` counts it as production code. Test-only helpers live in that file without `#[cfg(test)]`.

## Specs and issues

- `specs/` holds business requirements: what the software does, not how it is built, tested, or released. Build, CI, release, and test-harness mechanics live in this file.
- Gaps and missing features are GitHub issues with the `feature` label, not a backlog file.

## Build and release

- The repo is a pure Cargo workspace. Shared dependencies live in `[workspace.dependencies]`.
- `arrow` stays pinned to the version `exarrow-rs` uses, so the `.so` boundary shares one copy.
- Bump `[workspace.package].version` (SemVer) only for changes downstream users can observe: runtime behavior, SDK, macro, or CLI API, or the container image. The version is part of the ABI fingerprint, so every bump forces downstream UDF rebuilds. The pinned `exasol-udf-sdk` entry must track it, and the regenerated `Cargo.lock` is committed in the same PR.
- A pushed `vX.Y.Z` tag matching `Cargo.toml` triggers the crates.io release, published in dependency order: `exasol-udf-sdk`, `exasol-udf-macros`, `cargo-exasol-udf`. Publishing is the only irreversible step, so tag only after review and green CI.
- The SLC builds from the root `Dockerfile` into a shell-less image: no shell, package manager, or coreutils. A UDF that shells out fails. A UDF may link dynamically only against the staged surface (glibc, `libgcc_s`, `libstdc++`, NSS and resolver modules, OpenSSL 3, `libz`, `libbz2`, `libzstd`) and links anything else statically, for example `features = ["vendored"]` on a `-sys` crate. The glibc floor is in `crates/cargo-exasol-udf/slc-glibc-floor.txt` and is checked by `cargo exasol-udf validate`.

## CI

- On Ubuntu 24.04 runners, run `sudo sysctl -w kernel.apparmor_restrict_unprivileged_userns=0` before `docker run` of the Exasol DB. Otherwise every UDF reports `Internal error: VM crashed` (SQL state 22002), because AppArmor strips `CAP_SYS_ADMIN` from `nschroot` even under `--privileged`. Confirm with `sudo dmesg` (`apparmor="DENIED" ... comm="nschroot" capability=21`).

## UDF runtime behavior

- End `main()` of `exaudfclient` with `std::process::exit(0)`. A normal return joins the connect-back Tokio threads and delays exit by 10+ seconds, which ends in `SIGABRT` (Part:40).
- `EMIT_BUFFER_LIMIT_BYTES = 4_000_000` is a flush target, not a database wire limit. It is 4 million bytes, not 4 MiB, and matches the reference C++ SLC.
- `ctx.emit` buffers rows and flushes one `MT_EMIT` when the running byte estimate reaches the target. `EmitBuffer` updates that estimate on each `push`. Always flush at the end of `run()`. A single row above the target is still sent as one `MT_EMIT`, up to 2 GB.
- Every emitted row carries the `row_number` of its input row. Without it the database reads out of range and closes the session.
- Connect-back works for `SCALAR` and `SET` scripts and for both transports. The address is `<container-eth0-ip>:8563` from `ctx.cluster_ip()`. Never use `127.0.0.1` or the Docker host gateway, both end in `SIGABRT`.
- Connect-back logs in with CONNECTION-object credentials in its own transaction. Reads are always safe. A write-back must not conflict with the invoking query, or it blocks on WAIT FOR COMMIT and aborts with `SIGABRT` after about 11 seconds.
- `ExaConnection::query` collects the whole result in memory, so use it only for small results. For table-scale reads, stream: convert one fetched Arrow batch at a time to a `Vec<Value>` chunk, hand it to the caller, and drop the batch before converting the next. Use `conn.query_for_each(sql, |row| ctx.emit(row))`.
- The `ExaConnection` trait stays Arrow-free. Only `Vec<Value>` chunks cross the `.so` boundary, because Arrow `TypeId` is not stable across it.
- Keep three concepts apart: the Exasol CONNECTION object (credential store), the exarrow-rs session (the connect-back), and the cluster node IP (`ctx.cluster_ip()`).
- The ZMQ control channel is chosen by the database (`ipc://` single node, `tcp://` multi node), cannot be set via `SCRIPT_LANGUAGES`, and does not affect connect-back.

## Benchmarks

- The suite lives in `benches/` and `crates/exa-mock-db`. Commands, profiles, and cell names are in `benches/README.md`. `quick` is the dev loop. `full` is the evidence a performance PR quotes.
- Deltas inside the noise band (8% full, 15% quick) or with an interval crossing zero is no change.
- `bench-udfs` is an optional dependency of `exa-udf-runtime` behind the `bench` feature, never a dev-dependency, because it needs `emit-arrow` and that would unify into every test build.
- `scalar_emits_pt` gates that emitted rows land beside their input rows. Do not relax it.
- Describe engine behavior observably. Cite only the protobuf definition, the reference C++ SLC, and measurements.
