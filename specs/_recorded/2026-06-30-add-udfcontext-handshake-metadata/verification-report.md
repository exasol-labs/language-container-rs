# Verification Report: add-udfcontext-handshake-metadata

## Verdict: PASS

All 9 implementation tasks complete, code review PASS on every correctness property, and the full automated checklist — including the live-DB integration suite — is green. Version bumped 0.19.1 → 0.20.0 (MINOR: additive SDK surface + dead-code removal).

## Checklist Results

| Step | Command | Result |
|------|---------|--------|
| Build | `cargo build --release` / `cargo build` | Exit 0 |
| Unit tests | `cargo test --all-features` | 0 failures |
| Integration + E2E | `cargo test -p it --features integration` (docker-db 2025.2.1) | `db_roundtrip_all_scenarios`: 1 passed, 0 failed (69.68s) |
| Lint | `cargo clippy --all-targets --all-features -- -D warnings` | 0 warnings |
| Format | `cargo fmt --check` | No changes |

## Scenario Coverage

| Scenario | Test | Status |
|----------|------|--------|
| UdfContext exposes handshake identity/origin metadata (defaults) | `default_handshake_metadata_is_neutral` (unit) | PASS |
| UdfContext exposes per-instance memory limit (regression) | `default_memory_limit_is_zero` (unit) | PASS |
| Bridge surfaces handshake metadata to the UDF | `bridge_returns_handshake_metadata` (unit) | PASS |
| Handshake metadata carries no buffered connect-back credentials | `from_pb_decodes_handshake_metadata_without_conn_info` (unit) | PASS |
| End-to-end: live handshake values reach UDF code | `handshake_metadata_udf_emits_session_and_node` (integration) | PASS |

The E2E scenario is a genuine liveness gate: `session_id` and `node_count` asserted non-zero (neutral default is 0), `script_name` asserted to contain the registered name `HANDSHAKE_META` (neutral default empty). `node_id` validated as a parseable `u32` only, since it is 0-based and single-node Docker legitimately reports 0.

## Code Review Summary

PASS on all four correctness properties (defaulted non-feature-gated accessors; exact-value bridge overrides with `Some`/`None` preserved; proto `optional` present/absent preserved in `from_pb`; `conn_info` buffering removed while `ConnInfo`/`HostEvent::ConnInfo` retained for the on-demand MT_IMPORT path).

Notable sound deviation from the plan: the implementer grouped the 12 handshake values into a single owned `HandshakeMeta` constructor parameter (with `From<&UdfMeta>` + `Default`) rather than 12 positional args — this avoids an ~18-argument constructor and matches the "config struct" guardrail. Accepted.

Minor/informational (no action taken): speculative `Clone` derive on `HandshakeMeta` (harmless); mixed accessor/direct field reads in `HandshakeMeta::from` (forced by `UdfMeta` field visibility, not a defect); `meta_info` proto field 12 intentionally out of scope per plan.

## Ready for: `/speq:record add-udfcontext-handshake-metadata`

## Post-record amendment (2026-07-01): CI-only IT failure

The original verification (step 5.5) ran the integration suite **locally**, where `cargo test -p it` builds every fixture `.so` via the workspace `default-members`. That masked a gap: the CI "Build UDF .so artifacts (release)" step in `.github/workflows/ci.yml` builds fixtures from an **explicit `-p` allowlist**, and Task 2.8 added the `test-udfs/handshake-meta` fixture crate + the `handshake_metadata_udf_emits_session_and_node` IT scenario without adding the fixture to that allowlist.

Result: PR #40 was green everywhere except the IT matrix, which failed on all three DB versions (2025.1.11 / 2025.2.1 / 2026.1.0) with:

```
test db_roundtrip_all_scenarios ... FAILED
Error: reading UDF artifact .../target/release/libhandshake_meta.so
    No such file or directory (os error 2)
```

Fix (Phase 6):
- `.github/workflows/ci.yml` — added `-p handshake-meta` to the "Build UDF .so artifacts (release)" step so `libhandshake_meta.so` is built and uploaded for the integration matrix to download.
- `CLAUDE.md` (CI section) — added a guardrail so future `test-udfs/*` fixtures are wired into the CI allowlist, preventing a recurrence of this local-passes / CI-fails split.

No spec deltas: this is a build/CI mechanic, which per CLAUDE.md lives in `ci.yml`/`CLAUDE.md`, not the spec library.
