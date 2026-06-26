# Tasks: add-debug-output-redirect

## Phase 2: Implementation (Group A) — parse + vm_id/accessors
- [x] 2.1 Add `parse_debug_level` to `exa-udf-runtime/src/artifact.rs` (Task 1.1)
- [x] 2.2 Unit tests: present/absent/trailing-semicolon/unrecognised (Task 1.2)
- [x] 2.3 Re-export `parse_debug_level` from `exa-udf-runtime/src/lib.rs` (Task 1.3)
- [x] 2.4 Parse `vm_id` into `UdfMeta::from_pb` + public accessors for `session_id`/`node_id`/`vm_id` (Task 3.2)

## Phase 2: Implementation (Group B) — apply level + formatter tagging
- [x] 2.5 Apply resolved level post-handshake via `LevelFilter::set_max_level` in runtime `lib.rs` (Task 2.1/2.2)
- [x] 2.6 Formatter: pid/node_id/session_id (+vm_id) fields on every line in `exaudfclient/src/main.rs` (Task 3.1)
- [x] 2.7 Confirm/document per-write stderr flush (no userspace BufWriter) (Task 3.3)

## Phase 2: Implementation (Group C) — SDK logging surface + version bump
- [x] 2.8 Default `UdfContext::debug_level()` in `exasol-udf-sdk/src/context.rs` (Task 4.1)
- [x] 2.9 `udf_log!` macro in `exasol-udf-sdk` (Task 4.2)
- [x] 2.10 Implement `debug_level()` on host bridge/SingleCallContext in `rowset.rs` (Task 4.3) [expert]
- [x] 2.11 Bump workspace version + pinned sdk dep + Cargo.lock + ABI fingerprint/loader tests (Task 4.4)

## Phase 2: Implementation (Group D) — telemetry + emit/flush spans
- [x] 2.12 RSS from /proc/self/statm + expose EmitBuffer estimate/cumulative/counts (Task 5.1)
- [x] 2.13 debug-gated telemetry event at phase transitions + checkpoint (Task 5.2)
- [x] 2.14 Spans/events around push/push_batch + MT_EMIT flush (Task 5.3)
- [x] 2.15 Integration test: telemetry at debug, none at info (Task 5.4)

## Phase 2: Implementation (Group E) — docs
- [x] 2.16 Docs: SET SESSION SCRIPT OUTPUT ADDRESS as the redirect mechanism (Task 6.1)
- [x] 2.17 Docs: %udf_debug_level + udf_log!/ctx.debug_level, contrast with Python SLC (Task 6.2)

## Phase 3: Verification
- [x] 3.1 cargo test --workspace (unit) — green; E2E via scripts/ci-it-local.sh rc=0 (24 scenarios)
- [x] 3.2 cargo test — green (non-it green; it db_roundtrip ok under harness)
- [x] 3.3 cargo clippy --all-targets --all-features -- -D warnings — clean
- [x] 3.4 cargo fmt --check — clean
## Phase 4: Review fixes
- [x] 4.1 Create tests/debug_level.rs covering all in-process plan scenarios (blocker) [expert]
- [x] 4.2 Root span: enter at ERROR level so VM tags appear below INFO (should-fix)
- [x] 4.3 rowset.rs flush_count off-by-one in telemetry (should-fix)
- [x] 4.4 loader.rs: rename v5 test + fix v5 string literals to v6 (should-fix)
- [x] 4.5 docs/debugging.md: version 0.18 -> 0.19 (should-fix)
- [x] 4.6 run() doc comment: explain why LevelFilter::current reflects reload (nit)
