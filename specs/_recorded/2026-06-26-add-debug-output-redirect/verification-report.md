# Verification Report: add-debug-output-redirect

## Bottom Line

**PASS.** All implementation tasks complete, code review findings fixed, and both
the unit suite and the live integration/E2E harness are green. Workspace version
bumped `0.18.0 → 0.19.0` (additive `dyn UdfContext` vtable change; ABI version
`5 → 6`). Ready for `/speq:record`.

## Automated Checks

| Step | Command | Result |
|------|---------|--------|
| Unit tests | `cargo test --workspace` (non-`it`) | ✅ all green |
| Lint | `cargo clippy --all-targets --all-features -- -D warnings` | ✅ no issues |
| Format | `cargo fmt --check` | ✅ clean |
| Integration + E2E | `DB_MEM='4 GiB' MEM=12g SHM=2g scripts/ci-it-local.sh` | ✅ `rc=0`, `db_roundtrip_all_scenarios` ok (24 scenarios, live Exasol 2026.1.0 + real SLC) |

> Note: plain `cargo test --workspace` reports the `it` crate's `db_roundtrip`
> test as failed when `SLC_TARBALL` is unset — this is by design (the harness
> builds the tarball). Under `scripts/ci-it-local.sh` (which builds the SLC,
> boots the DB, and runs the `it` binary in external mode) it passes.

## Scenario Coverage

| Scenario | Test | Status |
|----------|------|--------|
| Debug level directive sets the runtime verbosity | `artifact.rs::parses_debug_level_with_default` (+3 unit tests) | ✅ |
| The resolved level changes the global max verbosity at runtime | `tests/debug_level.rs::resolved_level_sets_global_max_level` | ✅ |
| UDF code log calls reach the stderr stream | `tests/debug_level.rs::udf_log_macro_writes_to_stderr_when_permitted` + sdk `lib.rs` tests | ✅ (in-process stderr capture limitation documented) |
| The context exposes the resolved debug level to UDF code | `tests/debug_level.rs::context_reports_resolved_debug_level` | ✅ |
| Every runtime line is tagged with its origin VM | `tests/debug_level.rs::runtime_lines_carry_vm_tags` | ✅ |
| Runtime lines are flushed individually | `tests/debug_level.rs::runtime_lines_flushed_per_write` | ✅ |
| Memory and emit-buffer telemetry emitted at debug level | `rowset.rs::telemetry_emitted_at_debug_level_only` | ✅ |
| The emit and flush path is instrumented | `rowset.rs::emit_flush_path_instrumented` | ✅ |
| The DB redirect captures all UDF process output | Live `SET SESSION SCRIPT OUTPUT ADDRESS` is a DB-side mechanism; covered by the E2E roundtrip exercising real UDF stderr. Manual-test steps documented in `docs/debugging.md`. | ✅ (DB-owned; no SLC code path) |

## Deviation from Plan (accepted)

Plan Decision [2] specified `tracing::level_filters::LevelFilter::set_max_level(level)`
and rejected `tracing_subscriber::reload`. That API does **not exist** in `tracing 0.1`.
Implementation uses a `tracing_subscriber::reload`-wrapped `EnvFilter`, modified once
post-handshake — no new crate dependency (`reload` ships with `tracing-subscriber`'s
default `std` feature). `reload::Handle::modify` calls `rebuild_interest_cache()`, which
updates the global `MAX_LEVEL` atomic that `LevelFilter::current()` reads, so
`ctx.debug_level()` (which reads `current()`) correctly observes the resolved level.
The decision log should be updated to reflect the real mechanism during `/speq:record`.

## Code Review

One blocker (missing `tests/debug_level.rs` scenario coverage) and five
should-fix/nit findings were raised and **all fixed** (tasks 4.1–4.6): the integration
test file was created, the per-VM root span was lowered to `ERROR` level so tags appear
at any verbosity, a `flush_count` off-by-one in telemetry was corrected, stale `v5` ABI
strings in `loader.rs` were updated to `v6`, and the docs example version was bumped to
`0.19`.
