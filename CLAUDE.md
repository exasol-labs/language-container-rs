# Project Rules

**Spec-driven project using speq-skill.**

Project mission in: @specs/mission.md

## Specs & issues

- The spec library (`specs/`) holds the **business requirements** of the software — *what* it does, not *how* it's built, tested, or released. Build/CI/release/test-harness mechanics live in this file, not in specs.
- Gaps and missing features are tracked as **GitHub issues** (`feature` label), not a backlog file.

## Build & release

- Pure Cargo workspace (no Bazel); shared deps centralized in `[workspace.dependencies]`.
- `arrow` must stay pinned to the version `exarrow-rs` uses — one shared copy across the `.so` boundary.
- Bump `[workspace.package].version` (SemVer) only for changes observable by downstream users: runtime behaviour, SDK/macro/CLI API, the container image. Not for tooling, docs, benchmarks or test coverage. The version is part of the ABI fingerprint, so every bump forces downstream UDF rebuilds. The pinned `exasol-udf-sdk` entry in `[workspace.dependencies]` must track it; commit the regenerated `Cargo.lock` in the same PR.
- A pushed `vX.Y.Z` git tag (matching `Cargo.toml`) triggers the crates.io release; CI publishes in dependency order `exasol-udf-sdk` → `exasol-udf-macros` → `cargo-exasol-udf`, skipping versions already on the index (re-runs are idempotent). The publish is the only irreversible step — review + green CI before tagging.

## Exasol / tooling

- Use Exasol Docker images to run Integration Tests and E2E tests
- Use `exapump` for all Exasol interaction.
- DSNs must include `validateservercertificate=0` (self-signed Docker cert).
- Integration tests: `cargo test -p it --features integration`; they **fail** (not skip) if the Docker DB is unavailable. CI runs the version matrix `8.29.x / 2025.1.x / 2026.1.x`.

## CI (Ubuntu 24.04 runners)

- Run `sudo sysctl -w kernel.apparmor_restrict_unprivileged_userns=0` before `docker run` of the Exasol DB.
- Otherwise every UDF reports `Internal error: VM crashed` (SQL state 22002) — AppArmor strips `CAP_SYS_ADMIN` from `nschroot` even under `--privileged`. It is **not** memory/disk/kernel/glibc/UDF code. Green locally on Debian; confirm on the runner via `sudo dmesg` (`apparmor="DENIED" ... comm="nschroot" capability=21`).
- **Adding a `test-udfs/*` fixture requires wiring it into CI.** The "Build UDF .so artifacts (release)" step in `.github/workflows/ci.yml` builds fixtures via an explicit `-p <crate>` allowlist (NOT `default-members`), then uploads `target/release/lib*.so` for the integration matrix to download. A new fixture that an IT scenario `dlopen`s must be added to that `-p` list, or IT fails CI-only with `reading UDF artifact .../lib<name>.so: No such file or directory` while passing locally (local `cargo test -p it` builds the `.so` via `default-members`).

## Connect-back

- Both `SCALAR` and `SET` scripts support connect-back; choose whichever UDF type fits the logic.
- Address must be `<container-eth0-ip>:8563` via `ctx.cluster_ip()`; never `127.0.0.1` or the Docker host gateway (both → SIGABRT). `cluster_ip()` reads the first non-loopback IPv4 via `getifaddrs`; tests get it from `container_inner_ip()`.
- Connect-back is a plain SQL login using CONNECTION-object credentials, running in its own independent transaction. Read-only is always safe; write-back must not write-write/schema-conflict with the invoking query (else WAIT FOR COMMIT → deadlock abort, Part:40 SIGABRT ~T+11s).
- Transport (native binary vs WebSocket) is irrelevant — UDF type is the differentiator.

## exaudfclient lifecycle

- End `main()` with `std::process::exit(0)` — never return normally. A normal return joins the connect-back Tokio runtime threads, delaying exit 10+s → Part:40 `SIGABRT` ~T+11s.

## Emit buffering and wire limits

- `EMIT_BUFFER_LIMIT_BYTES = 4_000_000` is a **flush target**, not a DB-enforced wire limit. The value matches the reference C++ SLC's `SWIG_MAX_VAR_DATASIZE = 4_000_000` (4 million bytes, not 4 MiB). The DB accepts larger `MT_EMIT` messages.
- Every emitted row carries the `row_number` of the input row it came from; the engine needs it to place pass-through select-list columns (`SELECT id, f(x) FROM t`). An `MT_EMIT` without it makes the DB read out of range and closes the session.
- `ctx.emit` must **not** send a message per call. Buffer rows and flush to `MT_EMIT` only when the byte estimate reaches 4,000,000 bytes.
- **Always flush at end of `run()`** — even if the threshold was not reached. The architect rule: "beim buffern ist auch wichtig, das man flushed, wenn die Run Methode durch ist".
- A single row can be up to 2 GB — this limit cannot be avoided. A row that alone exceeds the 4,000,000-byte threshold must still be sent as a single-row `MT_EMIT` (no way to split it).
- `EmitBuffer` must maintain a running byte-size estimate updated on each `push`, not recomputed on flush.

## Connect-back streaming

- `ExaConnection::query` is **collect-all** — the entire result set materialises in memory as `Vec<Vec<Value>>`. Use it only for small, bounded result sets.
- For table-scale reads, use the streaming API: fetch Arrow batches one at a time, convert each batch → `Vec<Value>` chunk, yield/callback to the caller, then **drop the batch before fetching the next one**. The architect rule: "du musst resultset in batches lesen und dann gleich emitten".
- Never accumulate all `RecordBatch`es before converting — that creates two in-memory copies (Arrow + Value) of the entire result simultaneously.
- The `ExaConnection` trait (SDK/FFI boundary) must remain **Arrow-free**: only `Vec<Value>` chunks cross the `.so` boundary; Arrow `TypeId` is not stable across dynamic library boundaries.
- The natural consumer pattern is emit-as-you-read: `conn.query_for_each(sql, |row| ctx.emit(row))` — read a chunk, emit it, discard it, repeat.

## Unit test layout

- Unit tests MUST live in `<module>_tests.rs` beside `<module>.rs`, declared as the last item of `<module>.rs`:
  ```rust
  #[cfg(test)]
  #[path = "<module>_tests.rs"]
  mod tests;
  ```
- The file name MUST match `[0-9a-zA-Z_-]+[_-]tests.rs`. `cargo llvm-cov` excludes exactly that pattern from every report; any other name silently re-inflates the coverage percentage.
- The test module remains a child module of its parent, so `use super::*;` still reaches the parent's private items and its imports.
- A test-only helper (e.g. an accessor or a `to_pb`-style debug converter) that exists solely for tests belongs in the sibling `_tests.rs` file, not in the production module — add it there as `impl super::TypeName { ... }` (or a plain free fn), not gated by `#[cfg(test)]` since the whole file is already test-only via the `mod tests;` declaration.

## Runtime dlopen fixtures

- `exa-udf-runtime`'s integration tests load `test-udfs/*` cdylibs declared as its `[dev-dependencies]`; that edge is what builds and rebuilds them, and `tests/common/mod.rs` resolves the path (cargo-driven host runs only). A new fixture needs a dev-dependency entry, plus the CI `-p` allowlist if an IT scenario also loads it.
- `tests/emit_arrow_dlopen.rs` is behind the dev-only `emit-arrow-test` feature, so it runs only under `--features emit-arrow-test` or `--all-features` (what CI's coverage job uses). With any other flag set — including plain `--features emit-arrow` — it silently compiles out instead of failing.

## Benchmarks

- Suite in `benches/` and `crates/exa-mock-db`; commands, profiles and cell names in `benches/README.md`. `quick` is the loop, `full` is the evidence a performance PR quotes for both tiers.
- Tier 1 A/B is Criterion `--save-baseline` / `--baseline`; Tier 2 A/B is `udf-bench compare` over alternating runs. Deltas inside the band (8 % full, 15 % quick) or with an interval crossing zero are no change.
- A Tier 2 query returns one row and aggregates a UDF output column, so nothing streams to the client and the optimizer cannot skip the call.
- `bench-udfs` is an optional dependency of `exa-udf-runtime` behind the `bench` feature, never a dev-dependency (it needs `emit-arrow`, which would unify into every test build).
- CI runs only the Tier 1 smoke; it asserts row counts, a `row_number` per emitted row, and zero `MT_EMIT` messages over 4,000,000 bytes.
- `scalar_emits_pt` gates that emitted rows land beside their input rows; do not relax it.
- Describe engine behaviour observably; cite only the protobuf definition, the reference C++ SLC and measurements.

## Misc

- Keep the three "connection" concepts distinct: Exasol CONNECTION object (credential store) vs exarrow-rs session (the connect-back act) vs cluster node IP (`ctx.cluster_ip()`).
- The ZMQ control channel is DB-chosen (`ipc://` single-node, `tcp://` multi-node), not settable via `SCRIPT_LANGUAGES`, and has no effect on connect-back (always TCP to :8563).
- The SLC builds from one root `Dockerfile` (`rust:1.94-trixie` builder → `debian:trixie-slim` staging → `FROM scratch` artifact). The staged tree has no shell, package manager, or coreutils — a UDF that shells out to `/bin/sh` or a coreutils binary fails.
- The staged library surface is glibc + compatibility stubs, `libgcc_s`/`libstdc++`, the NSS/resolver modules, OpenSSL 3 (`libssl`, `libcrypto`, `ossl-modules`, `engines-3`), and `libz`/`libbz2`/`libzstd`, independent of whether `exaudfclient` itself links any of them (it links none of the OpenSSL/compression set). A UDF may link dynamically against this surface only; link anything else statically (e.g. `features = ["vendored"]` on a `-sys` crate). The glibc floor is `2.41`, committed in `crates/cargo-exasol-udf/slc-glibc-floor.txt` and checked by `cargo exasol-udf validate`.
