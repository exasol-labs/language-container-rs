# Plan: drop-unused-deps-and-dead-code

## Summary

Maintenance-only cleanup: remove unused dependencies, delete dead test fixtures, tighten internal `pub` surface, and strip debug scaffolding, capped with a PATCH bump to 0.15.1. No public API changes and no protocol behavior changes.

## Design

Skipped — this is a small maintenance change with no new feature or architectural decision. Every item is a removal, a visibility narrowing, or a manifest edit, each independently verifiable by `cargo build`/`cargo test`/`cargo clippy`. The only cross-file reasoning is the `HostAction` dead-variant removal and the `meta.rs` visibility narrowing, both confined to the internal `exa-zmq-protocol` crate.

## Features

| Feature | Status | Spec |
|---------|--------|------|
| workspace/bootstrap | CHANGED | `workspace/bootstrap/spec.md` |
| workspace/version | NEW (scenario) | `workspace/version/spec.md` |
| container/slim-image | CHANGED | `container/slim-image/spec.md` |

Internal code changes (dep removals, dead fixture deletions, visibility narrowing, debug scaffolding removal) are pure implementation — they do not alter any published public API or any behavior described by an existing scenario, so they need no spec delta beyond the three above.

## Dependencies

None added. Several removed (see Dead Code Removal). No prerequisite work — merged PR #22 already handled the prior cleanup batch.

## Implementation Tasks

Grouped by independence. Most tasks are mechanical manifest edits or whole-file deletions; only 3.1 and 3.2 require careful cross-file reasoning.

### Group 1 — Clean dependency removals (manifest-only)

- [ ] 1.1 Remove `anyhow = { workspace = true }` from `crates/exa-udf-runtime/Cargo.toml` — library crate; zero `.rs` uses (policy: `anyhow` for binaries only)
- [ ] 1.2 Remove `anyhow = { workspace = true }` from `crates/exaudfclient/Cargo.toml` — `main.rs` uses a hand-rolled `Exit` struct; no `anyhow` token in `.rs` files
- [ ] 1.3 Remove `prost-types = { workspace = true }` from `crates/exa-proto/Cargo.toml` — the proto file has no `import` statements; generated code has zero `prost_types` refs. Confirm with a clean rebuild (`cargo clean -p exa-proto && cargo build -p exa-proto`)
- [ ] 1.4 Remove the `indexmap = ">=2.0, <2.14"` entry from `[workspace.dependencies]` in the root `Cargo.toml` — declared but referenced by no member crate
- [ ] 1.5 Remove `arrow = { workspace = true }` from `test-udfs/connect-back-query/Cargo.toml` — appears only in a doc comment; delete the now-stale doc comment that referenced it

### Group 2 — Dead test fixture removals

- [ ] 2.1 Remove the `test-udfs/spike-connect/` crate directory and drop it from both `members` and `default-members` in the root `Cargo.toml` — `#[tokio::main]` spike binary, not a UDF lib; sole carrier of `exarrow-rs`/`arrow`/`tokio`/`anyhow` as direct deps among `test-udfs`
- [ ] 2.2 Fold the `Decimal` annotation variant into the macro annotation test, then remove the `test-udfs/annotated-double/` crate. Add a `#[exasol_udf(input(x: Decimal), emits(result: Decimal))]` case (plus a schema-string assertion) to `crates/exasol-udf-macros/tests/annotation.rs`, mirroring the existing `annotated_double_embeds_schema` assertion style. Then delete `test-udfs/annotated-double/` and drop it from `members` and `default-members`. Do NOT touch `test-udfs/annotated-fixture/` — its `annotated_double` second entry point is the one exercised by `crates/it/tests/db_roundtrip.rs`

### Group 3 — Internal dead `pub` shrink (exa-zmq-protocol + exa-udf-runtime)

- [ ] 3.1 Remove the 10 never-constructed `HostAction` variants from `crates/exa-zmq-protocol/src/messages.rs` — keep only `MetaRequest` and `PingReply`; delete `Info`, `MetaReply`, `EmitData`, `Next`, `DoneReply`, `CleanupReply`, `FinishedReply`, `CloseError`, `SingleCallReturn`, `UndefinedCall` and any now-dead match arms or helper code referencing them [expert]
- [ ] 3.2 Shrink the unused `pub` surface in `crates/exa-zmq-protocol`: remove `Protocol::cleanup_reply()`; narrow `Protocol::connection_id()`, `Protocol::phase()` + `Phase` enum (and its `lib.rs` re-export), `IterType` + `UdfMeta.input_iter`/`output_iter` + `iter_from_pb`/`iter_to_pb`, `ColumnMeta::to_pb()`/`UdfMeta::to_pb()`, and the `UdfMeta` fields `script_name`/`session_id`/`node_id`/`node_count` to `pub(crate)` or private. Items consumed only by the crate's own `src/tests.rs` MUST become `pub(crate)`, not be deleted. `iter_from_pb` is already called by `from_pb` so it stays (just drop `pub` if present) [expert]
- [ ] 3.3 Reduce `LoadedUdf::annotated_input_schema` and `LoadedUdf::annotated_output_schema` to `pub(crate)` in `crates/exa-udf-runtime/src/loader.rs` — consumed only by `schema_check.rs`

### Group 4 — Debug scaffolding + spec/Dockerfile reconciliation

- [ ] 4.1 Remove the getrandom probe `eprintln!("[slc] getrandom probe rc=…")` from `crates/exaudfclient/src/main.rs` and remove `libc = { workspace = true }` from `crates/exaudfclient/Cargo.toml` — the probe result drives no control flow and `libc` exists only for it
- [ ] 4.2 Remove the `udf_diag.log` file-logging path from `crates/exaudfclient/src/main.rs`, keeping the existing stderr `tracing-subscriber` fallback — spec is "stderr only; Exasol captures stderr". Do NOT touch the `/tmp/exaudf_started.txt` startup marker (grepped by `it/src/lib.rs:276`) or `cb_log`/`connect_back.rs` (deferred to a separate plan)
- [ ] 4.3 Delete `Dockerfile.debian` — referenced by no CI workflow or script; lacks the `/conf/hosts`+`/conf/resolv.conf` symlinks the DB integration requires, so it could not safely be promoted
- [ ] 4.4 Apply the `container/slim-image` spec delta: remove the stale "Alpine builder compiles the binary against musl" scenario and fix `rust:1.91-bookworm` → `rust:1.92-bookworm` in the builder-toolchain scenario. Verify `Dockerfile.alpine` and `rust-toolchain.toml` already agree on `1.92`

### Group 5 — Version bump (runs LAST, after all removals settle)

- [ ] 5.1 Bump `[workspace.package].version` from `0.15.0` to `0.15.1` and update the `exasol-udf-sdk` pin in `[workspace.dependencies]` to `version = "0.15.1"` in the root `Cargo.toml`
- [ ] 5.2 Regenerate `Cargo.lock` via `cargo build` and confirm the lock-file diff reflects both the version bump and the removed deps (no `indexmap`, no `prost-types`, fewer `anyhow`/`libc`/`tokio`/`exarrow-rs` edges from the deleted fixtures)

## Parallelization

| Parallel Group | Tasks |
|----------------|-------|
| Group A (manifest/file edits, independent) | 1.1, 1.2, 1.3, 1.4, 1.5, 4.1, 4.2, 4.3, 4.4 |
| Group B (fixture removals, touch root members list) | 2.1, 2.2 |
| Group C (internal code shrink) | 3.1, 3.2, 3.3 |
| Group D (version bump) | 5.1, 5.2 |

Sequential dependencies:
- Groups A, B, C may run concurrently, but all edit no overlapping files **except** the root `Cargo.toml` members/default-members list (touched by 1.4, 2.1, 2.2). Serialize the root-`Cargo.toml` edits (1.4 → 2.1 → 2.2 → 5.1) to avoid edit conflicts; everything else is free.
- Group D (5.1, 5.2) → runs after A, B, C complete, so the regenerated `Cargo.lock` captures every removed dependency edge in one pass.

## Dead Code Removal

| Type | Location | Reason |
|------|----------|--------|
| Dependency | `crates/exa-udf-runtime/Cargo.toml` `anyhow` | Zero uses; library crate (policy: anyhow for binaries only) |
| Dependency | `crates/exaudfclient/Cargo.toml` `anyhow` | `main.rs` uses hand-rolled `Exit`; no `anyhow` token |
| Dependency | `crates/exa-proto/Cargo.toml` `prost-types` | Proto has no imports; zero `prost_types` refs in generated code |
| Dependency | root `Cargo.toml` `[workspace.dependencies].indexmap` | Declared but referenced by no member crate |
| Dependency | `test-udfs/connect-back-query/Cargo.toml` `arrow` | Only in a doc comment; no code use |
| Dependency | `crates/exaudfclient/Cargo.toml` `libc` | Existed only for the getrandom probe being removed |
| Crate | `test-udfs/spike-connect/` | Spike binary, not a UDF; no test or script references it |
| Crate | `test-udfs/annotated-double/` | Decimal variant folded into macro test; no external `.so` loader |
| Enum variants | `crates/exa-zmq-protocol/src/messages.rs` `HostAction` (10 of 12) | Never constructed or matched outside their own definition |
| Method | `crates/exa-zmq-protocol/src/loop_.rs` `Protocol::cleanup_reply()` | Nothing calls it |
| `pub`→`pub(crate)`/private | `exa-zmq-protocol` `connection_id`/`phase`/`Phase`/`IterType`/`*_iter`/`iter_*_pb`/`*::to_pb`/`UdfMeta` fields | Internal-only or test-only — no external consumer |
| `pub`→`pub(crate)` | `crates/exa-udf-runtime/src/loader.rs` `annotated_input_schema`/`annotated_output_schema` | Consumed only by `schema_check.rs` |
| Code | `crates/exaudfclient/src/main.rs` getrandom probe | Result drives no control flow |
| Code | `crates/exaudfclient/src/main.rs` `udf_diag.log` file logging | Stderr fallback suffices; spec is stderr-only |
| File | `Dockerfile.debian` | Unshipped; missing required `/conf` symlinks for DB integration |
| Scenario | `specs/container/slim-image/spec.md` "Alpine builder compiles the binary against musl" | Contradicts the glibc `Dockerfile.alpine` |

Out of scope (do NOT remove): `cb_log`→`/tmp/cb_debug.txt` in `connect_back.rs` (deferred — test-coupled); `/tmp/exaudf_started.txt` startup marker (grepped by `it/src/lib.rs:276`); the stale `udf_trace.txt` entry in `dump_udf_logs()` (part of the deferred `cb_log` plan).

## Verification

### Scenario Coverage

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| Workspace Cargo.toml is well-formed (no `indexmap`) | Integration | `crates/it` / manual grep | `grep -n indexmap Cargo.toml` returns nothing; `cargo build` resolves clean |
| Workspace version is bumped to 0.15.1 | Unit (manifest assertion) | manual grep | `[workspace.package].version = "0.15.1"` and SDK pin `version = "0.15.1"` |
| Builder toolchain and glibc runtime (1.92) | Integration | `crates/it/tests/db_roundtrip.rs` (Alpine SLC build) | existing db-roundtrip suite (image still builds + passes) |
| Alpine builder compiles against musl (REMOVED) | n/a | removed scenario | no test — scenario deleted |
| Decimal annotation variant (folded from annotated-double) | Unit (compile + schema string) | `crates/exasol-udf-macros/tests/annotation.rs` | new `decimal_annotation_embeds_schema` (or similar) test |
| `annotated_double` second entry point still loads | Integration | `crates/it/tests/db_roundtrip.rs` | `annotated_fixture_two_entries_from_one_so` (unchanged, must still pass) |

The internal-visibility and dead-code removals are verified structurally by the compiler and clippy: if a removed item were still referenced, `cargo build`/`cargo clippy` would fail. The full existing `cargo test` + `cargo test -p it --features integration` suites guard against behavioral regressions.

### Manual Testing

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| Dep removals + version bump | `cargo build --release` | Exit 0; `Cargo.lock` no longer lists `indexmap` or `prost-types`; `spike-connect`/`annotated-double` absent |
| Dead-code shrink compiles | `cargo clippy --all-targets --all-features -- -D warnings` | 0 warnings (no dead-code/unused-import lints) |
| Macro Decimal fold | `cargo test -p exasol-udf-macros` | new Decimal annotation test passes |
| exaudfclient logging | `cargo build -p exaudfclient && ./target/release/exaudfclient` | usage message on stderr referencing `lang=rust`, non-zero exit; no `udf_diag.log` created, no `[slc] getrandom probe` line |
| Dockerfile.debian gone | `test ! -f Dockerfile.debian && echo OK` | `OK` |
| Slim image still builds (1.92) | `docker build -f Dockerfile.alpine -t lc-rs-slim:dev .` | build completes; binary at `/exaudf/exaudfclient` |
| Stale musl scenario gone | `grep -rn "compiles the binary against musl" specs/` | no match |
| Roundtrip suite intact | `cargo test -p it --features integration` | all scenarios pass (incl. `annotated_double` second entry point) |

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Build | `cargo build --release` | Exit 0 |
| Clean rebuild exa-proto | `cargo clean -p exa-proto && cargo build -p exa-proto` | Exit 0; no `prost_types` errors |
| Test | `cargo test` | 0 failures |
| Integration test | `cargo test -p it --features integration` | 0 failures (live Exasol Docker) |
| Lint | `cargo clippy --all-targets --all-features -- -D warnings` | 0 errors/warnings |
| Format | `cargo fmt --check` | No changes |
| Stale musl scenario removed | `grep -rn "against musl" specs/container/slim-image/` | No match |
| Version bumped | `grep -n 'version = "0.15.1"' Cargo.toml` | 2 matches (workspace.package + SDK pin) |
