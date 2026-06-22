# Verification Report: drop-unused-deps-and-dead-code

**Generated:** 2026-06-22

## Verdict

| Result | Details |
|--------|---------|
| **PASS** | All 17 implementation tasks complete; build/test/lint/format green; code review found no material defects. Maintenance-only cleanup capped at version 0.15.1, no public-API or protocol-behavior change. |

| Check | Status |
|-------|--------|
| Build | ✓ |
| Tests | ✓ |
| Lint | ✓ |
| Format | ✓ |
| Scenario Coverage | ✓ |
| Manual Tests | ✓ |
| Code Review | ✓ |

## Test Evidence

### Test Results

| Type | Run | Passed | Failed | Ignored |
|------|-----|--------|--------|---------|
| Unit + integration (`cargo test`) | all workspace crates | all | 0 | 2 (`cli.rs`, intentional) |

Notable suites: `exa-zmq-protocol` 19+7+2; `exa-udf-runtime` dispatch 2, connect_back 5; `exasol-udf-macros` annotation 7 (incl. new `decimal_annotation_embeds_schema`), trybuild 4; `exasol-udf-sdk` 9; `exaudfclient` 3.

**Note on dispatch test:** `crates/exa-udf-runtime/tests/dispatch.rs` initially failed with a fingerprint mismatch (`expected 0.15.1, found 0.15.0`) because it loads the prebuilt `target/debug/libscalar_double.so`, a stale artifact from before the version bump. Rebuilding the test-udf fixtures at 0.15.1 resolved it — both `scalar_dispatch_full_protocol` and `annotated_schema_mismatch_closes_session` pass. This is a stale-artifact quirk of local incremental builds, not a regression; CI builds fixtures fresh.

### Manual Tests

| Test | Command | Result |
|------|---------|--------|
| Dep removals + version bump | `cargo build --release` | ✓ Exit 0; builds clean at 0.15.1 |
| Clean rebuild exa-proto (no prost_types) | `cargo clean -p exa-proto && cargo build -p exa-proto` | ✓ Exit 0; no `prost_types` errors |
| exaudfclient logging | `./target/release/exaudfclient` | ✓ Usage references `lang=rust`; exit 1; no `[slc] getrandom probe` line; no `udf_diag.log` created |
| Dockerfile.debian gone | `test ! -f Dockerfile.debian` | ✓ OK |
| spike-connect / annotated-double gone | `ls test-udfs/` | ✓ both absent |
| Stale musl scenario gone (active spec) | `grep -rn "against musl" specs/container/slim-image/` | ✓ no match |
| Version bumped | `grep -c '0.15.1' Cargo.toml` | ✓ 2 matches (workspace.package + SDK pin) |

## Tool Evidence

### Linter

```
cargo clippy --all-targets --all-features -- -D warnings
Finished `dev` profile [unoptimized + debuginfo] target(s)
0 warnings, 0 errors
```

### Formatter

```
cargo fmt --check
(no output — exit 0)
```

## Scenario Coverage

| Domain | Feature | Scenario | Test Location | Test Name | Passes |
|--------|---------|----------|---------------|-----------|--------|
| workspace | bootstrap | Cargo.toml well-formed, no `indexmap` direct dep | manual grep | `grep -n indexmap Cargo.toml` → empty | Pass |
| workspace | version | Version bumped to 0.15.1 | manual grep | `grep -c '0.15.1' Cargo.toml` → 2 | Pass |
| container | slim-image | Builder toolchain and glibc runtime (1.92) | `specs/container/slim-image/spec.md` | spec line 24 `FROM rust:1.92-bookworm` | Pass |
| container | slim-image | Alpine builder compiles against musl (REMOVED) | — | scenario deleted from active spec; delta marked `DELTA:REMOVED` | Pass (removed) |
| (macros) | annotation | Decimal annotation variant (folded from annotated-double) | `crates/exasol-udf-macros/tests/annotation.rs` | `decimal_annotation_embeds_schema` | Pass |
| (it) | db-roundtrip | `annotated_double` second entry point still loads | `crates/it/tests/db_roundtrip.rs` | `annotated-fixture` entry (untouched) | Pass (build) |

## Notes

- **`indexmap` and `prost-types` remain in `Cargo.lock` as legitimate transitive dependencies** — `indexmap` via `arrow`/`arrow-json` and `petgraph`; `prost-types` via `prost-build` (exa-proto's build-dep). Removing them as *direct* manifest declarations was the correct and complete action (they were genuinely unreferenced direct deps). The plan's expectation that they would vanish from the lock entirely was over-optimistic; their transitive presence is correct and unavoidable.
- **`specs/` grep for "compiles the binary against musl" still matches in `specs/_recorded/...` and the plan's own delta file** — these are immutable recorded-plan archives and the `DELTA:REMOVED`-marked delta, both correct by design. The active permanent spec `specs/container/slim-image/spec.md` is clean.
- **`UdfMeta::script_name` intentionally left `pub`** (deviation from plan task 3.2): it has a genuine external consumer at `crates/exa-udf-runtime/src/lib.rs:72`. All other listed items were narrowed to `pub(crate)`/private as planned.
- **Integration suite (`cargo test -p it --features integration`) not run** — requires a live Exasol Docker container with the CI AppArmor workaround. The change is dep/visibility/scaffolding only with no protocol-path edits; all affected logic is covered by the passing unit + dispatch suites. Recommend running the live-DB suite in CI before release.
- Code review (Phase 4): no material defects. One cosmetic note — asymmetric `dead_code` attributes between the iter fields and node/session fields in `meta.rs` — judged not worth changing (both compile warning-free).
