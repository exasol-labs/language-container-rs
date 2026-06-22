# Decision Log: drop-unused-deps-and-dead-code

Date: 2026-06-22

## Interview

**Q:** Which cleanup groups are in scope?
**A:** All four groups: (1) clean dep removals (`anyhow`/`prost-types`/`indexmap`/`arrow` from various manifests); (2) dead test fixtures (`spike-connect`, `annotated-double`); (3) internal dead `pub` shrink (`HostAction` dead variants, `exa-zmq-protocol` unused pub surface, `LoadedUdf` visibility); (4) debug scaffolding + spec/Dockerfile reconciliation (getrandom probe, `udf_diag.log`, `Dockerfile.debian`, stale musl scenario in slim-image spec) — but NOT `cb_log`→`/tmp/cb_debug.txt`, which is deferred to a separate plan.

**Q:** What version bump?
**A:** PATCH bump 0.15.0 → 0.15.1. There is no public API change, but the user wants to push the leaner manifest to crates.io.

**Q:** Anything explicitly excluded beyond `cb_log`?
**A:** Yes — the `/tmp/exaudf_started.txt` startup marker is test-coupled (`dump_udf_logs()` greps for it at `it/src/lib.rs:276`), so it stays. The stale `udf_trace.txt` entry in `dump_udf_logs()` is also left for the deferred `cb_log` plan. No changes to `connect_back.rs` or `it/src/lib.rs`.

## Design Decisions

### [1] PATCH bump despite a crates.io publish

- **Decision:** Bump 0.15.0 → 0.15.1 (PATCH).
- **Alternatives:** MINOR bump (as the 0.14.0 and 0.15.0 releases used) — rejected because those releases removed public API surface; this one does not.
- **Rationale:** SemVer PATCH is correct when no public API of any published crate (`exasol-udf-sdk`, `exasol-udf-macros`, `cargo-exasol-udf`) changes. All removals here are unused deps, internal `pub(crate)` narrowing, dead fixtures, and debug scaffolding — none reachable by UDF authors.
- **Promotes to ADR:** no

### [2] Three spec deltas only; internal removals need no spec

- **Decision:** Author deltas for `workspace/bootstrap` (no `indexmap`), `workspace/version` (new 0.15.1 scenario), and `container/slim-image` (remove musl scenario, fix 1.91→1.92). All other items are pure implementation with no governing scenario.
- **Alternatives:** Add spec scenarios pinning every dep removal and visibility level — rejected as over-specification; the compiler and clippy already enforce these structurally.
- **Rationale:** Specs describe externally-observable behavior. Removing a never-used `pub` method or a dead enum variant changes nothing a scenario can observe, so a `-D warnings` clippy gate is the right enforcement, not a spec clause.
- **Promotes to ADR:** no

### [3] Fold the Decimal annotation variant into the macro test, not annotated-fixture

- **Decision:** Move `annotated-double`'s unique bit (the `#[exasol_udf(input(x: Decimal), emits(result: Decimal))]` annotation + non-null schema-pointer assertion) into `crates/exasol-udf-macros/tests/annotation.rs`, then delete the crate.
- **Alternatives:** Fold into `test-udfs/annotated-fixture/` — rejected because that crate's role is the DB-roundtrip two-entry-point fixture (`annotated`, `annotated_double` as i64), and adding a Decimal-typed third entry would muddy that fixture's purpose; the macro test already owns schema-string assertions.
- **Rationale:** `annotation.rs` is the canonical home for "macro emits correct ExaType schema for a given Rust type"; the Decimal case belongs there. Deleting the crate then removes a `.so` that no external test ever loaded.
- **Promotes to ADR:** no

### [4] Serialize root-Cargo.toml edits across parallel groups

- **Decision:** Tasks 1.4, 2.1, 2.2, and 5.1 all edit the root `Cargo.toml` members/version; run them in sequence even though their groups are otherwise parallel.
- **Alternatives:** Let all parallel agents edit the root manifest — rejected to avoid edit conflicts on the shared `members`/`default-members` lists.
- **Rationale:** A single shared file is the only contention point; everything else is conflict-free.
- **Promotes to ADR:** no

### [5] prost-types removal requires a clean rebuild to confirm

- **Decision:** After removing `prost-types`, run `cargo clean -p exa-proto && cargo build -p exa-proto` rather than trusting an incremental build.
- **Alternatives:** Plain `cargo build` — rejected because prost codegen output is cached in `OUT_DIR`; an incremental build could hide a regenerated dependency on `prost_types`.
- **Rationale:** The generated code is the actual consumer of `prost-types`; only a from-scratch codegen + compile proves the dependency is truly unused.
- **Promotes to ADR:** no

## Review Findings

<!-- Populated by speq-implement after code review. -->
