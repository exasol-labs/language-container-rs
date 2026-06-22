# Verification Report: add-memory-limit-metadata

## Verdict: PASS

All implementation tasks complete; all automated checks and the live-DB E2E suite are green. Ready to record.

## Summary

Surfaced `exascript_info.maximal_memory_limit` (bytes, per-UDF-instance) through the
handshake → `UdfMeta` → `HostContextBridge` → `UdfContext::memory_limit()` path,
mirroring the existing `node_count` decode and defaulted-accessor idioms. Purely
additive; no ABI/vtable surface, no proto change, no feature gate.

## Checklist

| Step | Command | Result |
|------|---------|--------|
| Build | `cargo build --release` | Exit 0 |
| Test (unit/integration) | `cargo test` | 139 passed, 4 ignored, 0 failed |
| Lint | `cargo clippy --all-targets --all-features -- -D warnings` | 0 warnings |
| Format | `cargo fmt --check` | No changes |
| Live-DB E2E | `DB_MEM='4 GiB' MEM=12g SHM=2g scripts/ci-it-local.sh` | rc=0; `db_roundtrip_all_scenarios` 1 passed, 0 failed |

## Scenario Coverage

| Scenario | Test | Result |
|----------|------|--------|
| Memory limit is surfaced from the handshake info response | `exa-zmq-protocol::meta::from_pb_carries_maximal_memory_limit` | pass |
| UdfContext exposes the per-instance memory limit (default) | `exasol-udf-sdk::context::default_memory_limit_is_zero` | pass |
| UdfContext exposes the per-instance memory limit (host override) | `exa-udf-runtime::rowset::bridge_returns_memory_limit` | pass |

## Notes

- `cargo test` initially showed 2 failures in `exa-udf-runtime/tests/dispatch.rs`
  (`expected 0.15.1, found 0.15.0` fingerprint mismatch). Root cause: stale
  `target/debug` fixture `.so`s from the 0.15.0 era; rebuilding the workspace
  refreshed them and both tests pass. Not a repo defect — CI builds fixtures fresh.
- Version bumped `0.15.1` → `0.16.0` (minor; new public `UdfContext::memory_limit()`
  accessor). `[workspace.dependencies].exasol-udf-sdk` pin and `Cargo.lock` synced.
