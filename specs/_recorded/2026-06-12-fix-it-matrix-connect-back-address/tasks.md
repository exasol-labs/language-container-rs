# Tasks: fix-it-matrix-connect-back-address

## Phase 2: Implementation (Group A — IT harness fix)
- [x] 2.1 crates/it/src/lib.rs: connect_back_sql_address() always returns <container_inner_ip()>:8563 (both modes), never loopback, errors on resolution failure; update doc comment
- [x] 2.2 crates/it/tests/db_roundtrip.rs: run python3_connect_back diagnostic on a throwaway harness.connect() connection so a VM crash cannot poison the shared conn

## Phase 2: Implementation (Group B — deps + CI)
- [x] 2.3 Cargo.toml: exarrow-rs 0.12.5 → 0.12.7; cargo +1.91 update -p exarrow-rs; cargo +1.91 build clean
- [x] 2.4 .github/workflows/ci.yml: remove Checkout exarrow-rs step (218-222) and build-contexts exarrow-rs line (235)

## Phase 2: Implementation (Group C — docs/branding)
- [x] 2.5 README.md: delete Status: Alpha badge (line 6); rename prose "Script Language Container" → "Language Container" (lines 11, 19)
- [x] 2.6 specs/mission.md + specs/design.md (+ other non-_recorded prose): rename spelled-out "Script Language Container" → "Language Container"; keep SLC abbrev, SQL identifiers, image names, glossary acronym definitions, _recorded archives

## Phase 2: Implementation (Group D — spec deltas)
- [x] 2.7 specs/_plans/.../integration/connect-back/spec.md: rewrite Background + the 4 CB_SELF address parentheticals → both modes use <container-eth0-ip>:8563; loopback/CoreDB proxy forbidden
- [x] 2.8 specs/_plans/.../integration/db-roundtrip/spec.md: add Background + scenario for isolated throwaway-connection diagnostic

## Phase 3: Verification
- [x] 3.1 cargo +1.91 build clean; fmt --check clean; clippy clean
- [x] 3.2 Causation confirmed from failing CI run 27425472641 log: python3_connect_back + double_it share the SAME Session ID (shared-session poisoning). [Used existing CI evidence rather than burning a second local container cycle to reproduce the red state.]
- [x] 3.3 Built .so in rust:1.91-bookworm (glibc 2.36); rebuilt SLC image with exarrow 0.12.7; started exasol-db (2026.1.0); recompiled it-runner with fix
- [x] 3.4 Ran it-runner external mode (EXASOL_HOST=localhost EXASOL_PORT=18563 BUCKETFS_PORT=12581) → 1 passed; 0 failed; ALL 12 scenarios ok (incl. double_it + all connect-back); CB addr = 172.17.0.2:8563
- [x] 3.5 Code review done (9 findings; 2 in-scope fixed: DB_PORT magic literal, stale exarrow specs; 2 pre-existing noted)
- [x] 3.6 Verification report written
