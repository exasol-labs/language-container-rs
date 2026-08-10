# Plan: fix-glibc-cdylib-build-model

## Summary

Reconcile the spec library, docs, and build tooling to one build/deploy model — a glibc-dynamic cdylib `.so` built by a plain host `cargo build --release` — replacing the permanently-unbuildable "fully-static musl `.so`" claim (issue #80). Repoint `cargo exasol-udf build` to a host glibc build with an optional `--target <triple>` override, and delete the dead musl toolchain and target-spec config.

## Design

### Context

The specs and docs describe every UDF artifact as a "fully-static musl `.so`", but the shipped system builds and loads glibc-dynamic cdylibs. The musl path is not merely stale — it is unbuildable: a musl target defaults `crt-static` to true, so `rustc` emits no cdylib and errors `cannot produce cdylib ... target x86_64-unknown-linux-musl does not support these crate types`. `cargo exasol-udf build` therefore fails on every crate its own `new` scaffolds (`crate-type = ["cdylib"]`). The breakage shipped undetected because all three build-invoking tests in `crates/cargo-exasol-udf/tests/build.rs` are `#[ignore]`d. CI already builds fixtures the glibc way (`cargo build --release -p <crate>`, artifact `target/release/lib*.so`), so the working model exists — only the specs, docs, and the CLI default lag it.

- **Goals** — one consistent statement of the build/deploy model across specs and docs; a working end-to-end `cargo exasol-udf build`; removal of the dead musl toolchain/config; a test that gates the build subcommand against silent regression.
- **Non-Goals** — cross-compilation provisioning; the arm64/Exasol-Personal work (owned by PR #79); the `cargo exaudf` vs `cargo exasol-udf` command-name mismatch (recorded as a separate follow-up); `specs/mission.md` (already reconciled via `/speq:mission`).

### Decision

Adopt the glibc-dynamic cdylib as the single UDF artifact model. `cargo exasol-udf build` defaults to a plain host `cargo build --release` and prints `target/release/lib<crate>.so`; an optional `--target <triple>` restores the per-target artifact path `target/<triple>/release/lib<crate>.so` for a native build on a host with that target installed. No `rustup target add` auto-install: a host glibc build needs no extra target.

#### Architecture

```
author crate (crate-type = ["cdylib"])
        │  cargo exasol-udf build
        ▼
  default: cargo build --release          →  target/release/lib<crate>.so   (glibc-dynamic cdylib)
  override: cargo build --release
            --target <triple>              →  target/<triple>/release/lib<crate>.so
        │  upload to BucketFS + CREATE SCRIPT %udf_object
        ▼
  slim SLC image (bundled glibc runtime) dlopens the cdylib
```

#### Patterns

| Pattern | Where | Why |
|---------|-------|-----|
| Sensible default over required flag | `build.rs` target selection | The common case (host glibc) needs no flag; `--target` is the escape hatch, per design-philosophy "a config parameter is a decision the module declined to make" |
| Un-ignore a gating test | `tests/build.rs` | A `#[ignore]`d suite let a fully-broken subcommand ship; the glibc default build needs no special toolchain, so it can run unignored in CI |

### Consequences

| Decision | Alternatives Considered | Rationale |
|----------|------------------------|-----------|
| Host glibc `cargo build --release` default | Keep musl default | musl cdylib is unbuildable; glibc matches the shipped SLC runtime and CI |
| Keep `--target <triple>` override | Drop `--target` entirely | Preserves a native build for a non-default host target without provisioning cross toolchains |
| Remove musl toolchain/config in this plan | Defer to PR #79's arm64 work | #79 depends on #80 and rebases after it; #80 owns the build-model reconciliation |

## Features

| Feature | Status | Spec |
|---------|--------|------|
| tools/cargo-exaudf | CHANGED | `tools/cargo-exaudf/spec.md` |
| examples/test-udfs | CHANGED | `examples/test-udfs/spec.md` |
| examples/test-udfs-timestamps | CHANGED | `examples/test-udfs-timestamps/spec.md` |
| examples/test-udfs-connect-back | CHANGED | `examples/test-udfs-connect-back/spec.md` |

## Impact

Authors build with `cargo exasol-udf build` (host glibc, no musl target install) and upload `target/release/lib<crate>.so` instead of `target/x86_64-unknown-linux-musl/release/...`. This fixes a `build` subcommand that could not produce any artifact. No runtime/wire behavior changes; the SLC already loads glibc cdylibs. `--target <triple>` remains for a native build on a non-default host target. No migration for already-deployed `.so`s.

## Dependencies

PR #79 (`feat/add-arm64-support`) declares `Depends on #80`. This plan lands first and solely owns the musl→glibc build-model reconciliation. On rebase, #79 renumbers its arm64 ADR `024`→`025` and drops the content this plan supersedes (see decision-log entry [5]).

## Implementation Tasks

Detailed, ordered checklist with `[expert]` tags in `tasks.md`. Groups:

1. **CLI build model (code)** — repoint `build.rs` to host glibc default + `--target <triple>` override + `target/release` path; delete `MUSL_TARGET` and `ensure_musl_target`; repoint the `new` scaffold's `exasol-udf-sdk`/`exasol-udf-macros` pins to the current SDK line; rewrite `tests/build.rs` (un-ignore the default-build and host-triple override tests, each patching the scaffold against the local SDK). `[expert]`
2. **Config/toolchain/artifact removal** — Cargo.toml description; `rust-toolchain.toml` targets line; `.cargo/config.toml` musl stanza; `targets/*.json` + Dockerfile.alpine `COPY targets/` line.
3. **Docs + architecture.md reconciliation** — README, `writing-a-udf.md`, `cargo-ecosystem.md`, and `specs/architecture.md` L41/L91. (`installation.md` carries no musl/support-matrix framing on `main`; left to PR #79.)

## Parallelization

| Parallel Group | Tasks |
|----------------|-------|
| Group A | 1.1, 1.2, 1.3 (tests/build.rs + build.rs + new.rs — sequential within) |
| Group B | 2.1, 2.2, 2.3, 2.4 |
| Group C | 3.1, 3.2, 3.3, 3.4 |

Sequential dependencies: none across groups — A (code), B (config), C (docs/reference) touch disjoint files and may run concurrently.

## Dead Code Removal

| Type | Location | Reason |
|------|----------|--------|
| Const + fn | `crates/cargo-exasol-udf/src/build.rs` (`MUSL_TARGET`, `ensure_musl_target`) | Host glibc build needs no fixed triple or `rustup target add` |
| Test | `crates/cargo-exasol-udf/tests/build.rs` (`build_installs_missing_target`) | Tests the removed auto-install behavior |
| Config line | `rust-toolchain.toml` (`targets = ["x86_64-unknown-linux-musl"]`) | No component builds for musl |
| Config stanza | `.cargo/config.toml` (`[target.x86_64-unknown-linux-musl] linker`) | musl linker no longer used |
| File | `targets/x86_64-unknown-linux-musl-dylib.json` | Vestigial custom target spec; no rustc consumer (overlaps PR #79 P7) |
| Dockerfile line | `Dockerfile.alpine` (`COPY targets/ ./targets/`) | Sole consumer of the removed JSON |

NOTE: `about.toml:24` still pins `targets = ["x86_64-unknown-linux-musl"]`, consumed by the CI license step (`dist/generate-licenses.sh`, ci.yml:277-278) that produces `THIRD-PARTY-LICENSES.md`. Its repoint is DEFERRED to PR #79's P2 license task, which rewrites `about.toml` to list gnu+musl for both arches; #80 must not collide with that rewrite. The bundle stays computed for the abandoned musl target until #79 lands — a compliance follow-up tracked with #79, not a #80 CI blocker (`cargo-about` resolves target cfg without the rustup component). Mirrors task 2.4's `targets/*.json` overlap deferral; coordination recorded in decision-log [5].

## Verification

### Scenario Coverage

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| cargo-exaudf: build produces a loadable host cdylib | Integration | `crates/cargo-exasol-udf/tests/build.rs` | `build_produces_host_cdylib` (un-ignored) |
| cargo-exaudf: build honors an explicit target override | Integration | `crates/cargo-exasol-udf/tests/build.rs` | `build_honors_target_override` (runs in CI using the host triple `x86_64-unknown-linux-gnu` as `--target`) |
| cargo-exaudf: build verifies the artifact exports at least one named entry point | Integration | `crates/cargo-exasol-udf/tests/build.rs` | `build_produces_host_cdylib` (asserts a non-empty `__exa_udf_entry_<NAME>` set) |
| cargo-exaudf: new scaffolds a buildable UDF crate | Integration | `crates/cargo-exasol-udf/tests/build.rs` | `build_produces_host_cdylib` (scaffolds via `new`, patches the scaffold to the local SDK, builds it — proving the scaffold pins compile against the current SDK) |
| test-udfs: json-parse extracts a field using serde_json | Integration | `crates/it/tests/db_roundtrip.rs` | `db_roundtrip_all_scenarios` → `json_parse_extracts_name` |
| test-udfs: annotated-double declares its schema via the typed annotation | Integration | `crates/it/tests/db_roundtrip.rs` | `db_roundtrip_all_scenarios` → `annotated_fixture_two_entries_from_one_so` |
| test-udfs: annotated-fixture exports two named entry points from one .so | Integration | `crates/it/tests/db_roundtrip.rs` | `db_roundtrip_all_scenarios` → `annotated_fixture_two_entries_from_one_so` |
| test-udfs: emit-arrow-batch emits a manually built Arrow RecordBatch | Integration | `crates/it/tests/db_roundtrip.rs` | `db_roundtrip_all_scenarios` → `emit_arrow_batch_roundtrips` |
| test-udfs: set-sum aggregates a group and returns one value | Integration | `crates/it/tests/db_roundtrip.rs` | `db_roundtrip_all_scenarios` → set_sum group-spanning scenario |
| test-udfs: emit-k emits a variable number of rows per input row | Integration | `crates/it/tests/db_roundtrip.rs` | `db_roundtrip_all_scenarios` → emit_k zero/one/many scenario |
| test-udfs-timestamps: timestamp-add-second adds one second | Integration | `crates/it/tests/db_roundtrip.rs` | `db_roundtrip_all_scenarios` → `timestamp_arithmetic_roundtrips` |
| test-udfs-timestamps: timestamp-now returns local wall-clock time | Integration | `crates/it/tests/db_roundtrip.rs` | `db_roundtrip_all_scenarios` → `udf_local_time_matches_session_tz` |
| test-udfs-timestamps: timestamp-passthrough re-returns a TIMESTAMP unchanged | Integration | `crates/it/tests/db_roundtrip.rs` | `db_roundtrip_all_scenarios` → `timestamp_precision_matrix_roundtrips` |
| test-udfs-connect-back: connect-back-query emits a fetched value | Integration | `crates/it/tests/db_roundtrip.rs` | `db_roundtrip_all_scenarios` → `connect_back_udf_queries_and_emits` |
| test-udfs-connect-back: connect-back-insert writes rows during run | Integration | `crates/it/tests/db_roundtrip.rs` | `db_roundtrip_all_scenarios` → `connect_back_dml_inserts_visible_via_exapump` |
| test-udfs-connect-back: connect-back-scalar returns a fetched value | Integration | `crates/it/tests/db_roundtrip.rs` | `db_roundtrip_all_scenarios` → `connect_back_scalar_queries_and_returns` |

The example-fixture scenarios changed only their spec wording (drop the musl triple); the existing `it` scenarios already prove each fixture builds as a glibc cdylib and behaves correctly. The CI "Build UDF .so artifacts (release)" step builds every fixture via `cargo build --release -p <crate>` and uploads `target/release/lib*.so`, which is the glibc-cdylib compile proof.

### Manual Testing

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| tools/cargo-exaudf | `cargo exasol-udf new /tmp/demo-udf && cargo exasol-udf build /tmp/demo-udf` | Prints `/tmp/demo-udf/target/release/libdemo_udf.so`; file exists and exports `__exa_udf_entry_*` |
| tools/cargo-exaudf | `cargo exasol-udf validate /tmp/demo-udf/target/release/libdemo_udf.so` | Exit 0; reports the discovered UDF name as ABI-compatible |
| examples/test-udfs | `cargo build --release -p scalar-double -p set-filter -p json-parse` | `target/release/libscalar_double.so` etc. produced |
| examples/test-udfs (live DB) | `cargo test -p it --features integration` | `db_roundtrip_all_scenarios` passes; 0 failures |

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Build | `cargo build --release` | Exit 0 |
| Test | `cargo test` | 0 failures |
| CLI build test | `cargo test -p cargo-exasol-udf --test build` | `build_produces_host_cdylib` and `build_honors_target_override` pass (no longer ignored) |
| Integration | `cargo test -p it --features integration` | 0 failures |
| Lint | `cargo clippy --all-targets --all-features -- -D warnings` | 0 warnings |
| Format | `cargo fmt --check` | No changes |
