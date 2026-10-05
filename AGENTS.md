# AGENTS.md

Spec-driven development with mission in: @specs/mission.md

## Testing

- Integration and E2E tests run against a local Exasol Docker database. Start the container yourself, do not ask the user.
- Tests must fail, not skip, when Exasol is unavailable.
- Connection strings must set `validateservercertificate=0`, because the Docker image uses a self-signed certificate.

Project specifics:

- Use `exapump` for all Exasol interaction.
- Integration tests run with `cargo test -p it --features integration`. CI runs the version matrix `8.29.x / 2025.1.x / 2026.1.x`.
- A new `test-udfs/*` fixture needs a workspace `members` entry, and its crate name must equal its directory name. `scripts/build-test-udfs.sh` builds each `test-udfs/*/` as `-p <dirname>` for CI, so a mismatch fails only in CI with `reading UDF artifact .../lib<name>.so: No such file or directory`. A fixture that `exa-udf-runtime`'s own tests `dlopen` also needs an entry in that crate's `[dev-dependencies]`.
- `tests/emit_arrow_dlopen.rs` runs only under `--features emit-arrow-test` or `--all-features`. With any other flag set it compiles out silently.

## Code quality

- `cargo fmt --all -- --check` and `cargo clippy --workspace --all-targets --all-features -- -D warnings` must pass before committing. These are the CI commands. Without `--workspace` and `--all-features`, clippy skips the non-default members and every feature-gated module.

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
- Bump `[workspace.package].version` (SemVer) only for changes downstream users can observe: runtime behavior, SDK, macro, or CLI API, or the container image. Do not bump it for tooling, docs, benchmarks, or test coverage. The version is part of the ABI fingerprint, so every bump forces downstream UDF rebuilds. The pinned `exasol-udf-sdk` entry must track it, and the regenerated `Cargo.lock` is committed in the same PR.
- Merging a version bump to `main` releases it. After the full CI pipeline is green, the `release` job creates the `vX.Y.Z` tag, the GitHub Release, and the crates.io publish in dependency order: `exasol-udf-sdk`, `exasol-udf-macros`, `cargo-exasol-udf`. Never push a release tag by hand. The job releases only when the tag does not exist yet, so a hand-pushed tag skips the release. Publishing is the only irreversible step, so merge a bump only after review.
- The SLC builds from the root `Dockerfile` into a shell-less image: no shell, package manager, or coreutils. A UDF that shells out fails. A UDF may link dynamically only against the staged surface (glibc, `libgcc_s`, `libstdc++`, NSS and resolver modules, OpenSSL 3, `libz`, `libbz2`, `libzstd`) and links anything else statically, for example `features = ["vendored"]` on a `-sys` crate. The glibc floor is in `crates/cargo-exasol-udf/slc-glibc-floor.txt` and is checked by `cargo exasol-udf validate`.

## CI

- On Ubuntu 24.04 runners, run `sudo sysctl -w kernel.apparmor_restrict_unprivileged_userns=0` before `docker run` of the Exasol DB. Otherwise every UDF reports `Internal error: VM crashed` (SQL state 22002), because AppArmor strips `CAP_SYS_ADMIN` from `nschroot` even under `--privileged`. It is not memory, disk, kernel, glibc, or UDF code. Confirm with `sudo dmesg` (`apparmor="DENIED" ... comm="nschroot" capability=21`).

## UDF runtime behavior

- End `main()` of `exaudfclient` with `std::process::exit(0)`. A normal return joins the connect-back Tokio threads and delays exit by 10+ seconds, which ends in `SIGABRT` (Part:40).
- `EMIT_BUFFER_LIMIT_BYTES = 4_000_000` is a flush target, not a database wire limit. It is 4 million bytes, not 4 MiB, and matches the reference C++ SLC.
- `ctx.emit` buffers rows and flushes one `MT_EMIT` when the running byte estimate reaches the target. `EmitBuffer` updates that estimate on each `push`. Always flush at the end of `run()`. A single row above the target is still sent as one `MT_EMIT`, up to 2 GB.
- Every emitted row carries the `row_number` of its input row. The database uses it to place pass-through select-list columns (`SELECT id, f(x) FROM t`). Without it the database reads out of range and closes the session.
- Connect-back works for `SCALAR` and `SET` scripts, whichever transport the invoking client session uses. Address it to `<node-ip>:8563` from `ctx.cluster_ip()`, which returns the first non-loopback IPv4 of the node running the UDF. Tests get it from `container_inner_ip()`.
- Connect-back itself uses the native protocol. Over the WebSocket transport, exarrow-rs returns duplicated and missing rows for wide results.
- Connect-back logs in with CONNECTION-object credentials in its own transaction. Reads are always safe. A write-back must not conflict with the invoking query, or it blocks on WAIT FOR COMMIT and aborts with `SIGABRT` after about 11 seconds.
- `ExaConnection::query` collects the whole result as `Value` rows, so use it only for small results. `conn.query_for_each(sql, |row| ctx.emit(row))` converts and drops one Arrow batch at a time, so the whole result never exists as `Value` rows. It still fetches every batch before the first callback, so peak memory is the whole result in Arrow form.
- The `ExaConnection` trait stays Arrow-free. Only `Vec<Value>` chunks cross the `.so` boundary, because Arrow `TypeId` is not stable across it.
- Keep three concepts apart: the Exasol CONNECTION object (credential store), the exarrow-rs session (the connect-back), and the cluster node IP (`ctx.cluster_ip()`).
- The ZMQ control channel is chosen by the database (`ipc://` single node, `tcp://` multi node), cannot be set via `SCRIPT_LANGUAGES`, and does not affect connect-back.

## Benchmarks

- The suite lives in `benches/` and `crates/exa-mock-db`. Commands, profiles, and cell names are in `benches/README.md`. `quick` is the dev loop. `full` is the evidence a performance PR quotes.
- Deltas inside the noise band (8% full, 15% quick) or with an interval crossing zero are no change.
- `bench-udfs` is an optional dependency of `exa-udf-runtime` behind the `bench` feature, never a dev-dependency, because it needs `emit-arrow` and that would unify into every test build.
- `scalar_emits_pt` gates that emitted rows land beside their input rows. Do not relax it.
- Describe engine behavior observably. Cite only the protobuf definition, the reference C++ SLC, and measurements.
