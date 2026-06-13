# Plan: fix-it-matrix-connect-back-address

## Summary

Make the `integration` IT matrix green by fixing its single real defect: the
connect-back address used in **external mode**. `connect_back_sql_address()`
returns `localhost:8563` when `EXASOL_HOST=localhost` (the CI and local-repro
config), which resolves to `127.0.0.1` → Exasol's CoreDB proxy → a VM SIGABRT.
Because every scenario shares one connection, that crash poisons the session and
the *next* scenario (`double_it`) fails on the dead VM — which issue #2
misdiagnosed as an independent fatal scalar-UDF crash. There is one root cause,
not two. Also: harden the non-fatal Python3 diagnostic so it can never poison
the shared connection; bump `exarrow-rs` to the latest crates.io release; remove
vestigial CI exarrow-rs source injection; drop the README alpha badge; and
rename user-facing prose "Script Language Container" → "Language Container".

## Design

### Context

The new exarrow-style CI pipeline (PR #1, merged) fails `integration` on all
three DB versions, so `release` (`needs: [integration]`) never runs. The failing
run (27425472641) integration log is the key evidence:

```
scenario python3_connect_back FAILED: ... VM crashed (Session: 1867805944649744384)
Error: query: SELECT TO_CHAR(double_it(21)) ... VM crashed (Session: 1867805944649744384)
```

Both failures carry the **same DB Session ID**. All scenarios run sequentially
on one shared `Connection` (`crates/it/tests/db_roundtrip.rs:31`, module comment
8–10). Sequence:

1. `sanity_select_one` → ok.
2. `connect_back_python3_queries_and_emits` (db_roundtrip.rs:44) creates
   `CONNECTION CB_SELF_PY TO '{cb_addr}'` with
   `cb_addr = harness.connect_back_sql_address()`. In external mode that is
   `localhost:8563` → `127.0.0.1` → CoreDB proxy, which links the connect-back
   session to the invoking SQL worker (Part:40) → **VM SIGABRT**
   (`CLAUDE.md` "Address rules"). Caught as "non-fatal" (db_roundtrip.rs:46) —
   but the VM crash **poisons the shared session**.
3. `scalar_double_returns_42` (db_roundtrip.rs:82) runs on that dead session →
   "VM crashed (same Session)" → test aborts.

The misconception originates in plan `fix-connect-back-version-matrix`
(2026-06-10), whose `connect_back_sql_address()` task assumed external mode
means a *remote* cluster with "no container to docker exec," so it returned
`host:db_port`. But the IT suite's external mode only ever targets a **local
Docker container named `exasol-db`** (CI sets `EXASOL_HOST=localhost`;
`crates/it/src/lib.rs:115` hardcodes `container_name="exasol-db"`;
`exec_in_container` (lib.rs:310) already falls back to `docker exec exasol-db`).
So a container IP **is** obtainable, and `localhost:8563` is the one address
that crashes.

This matches the user's framing: connect-back is a regular external client
session to the DB node's real SQL listener; the UDF runs inside a container that
is **not** the DB, so the correct address is the DB container's own `eth0` IP,
never loopback.

- **Goals**
  - External-mode connect-back uses `<container-eth0-ip>:8563` (via
    `container_inner_ip()`), identical to testcontainers mode. Never loopback.
  - A non-fatal diagnostic scenario can never poison the shared connection.
  - Latest `exarrow-rs` from crates.io; remove dead CI exarrow-rs injection.
  - README/specs prose no longer brands the project a "Script Language Container".
- **Non-Goals**
  - The rowset row-major codec (`crates/exa-udf-runtime/src/rowset.rs`) — the
    evidence shows the scalar path is not the cause; left untouched. If
    `double_it` still fails on a *healthy* session after the fix, re-open.
  - Any `127.0.0.1`/CoreDB-proxy connect-back path.
  - A genuine remote-cluster (non-Docker) external mode — not a current use; a
    future enhancement (explicit CB-address env) would cover it. Documented.

### Decision

`Harness::connect_back_sql_address()` (`crates/it/src/lib.rs:269`) always
resolves the container `eth0` IP via `container_inner_ip()` and returns
`<ip>:8563`, in **both** testcontainers and external mode. It must never return
a loopback address; if IP resolution fails it errors loudly rather than falling
back to `localhost`.

The Python3 connect-back diagnostic (`db_roundtrip.rs:44`) runs on a
**throwaway** `harness.connect()` connection (the `CONNECTION`/`SCRIPT` objects
it creates are DB-global and persist regardless of session), so a VM crash there
cannot poison the main `conn`. The main connection stays pristine for the real
assertions.

`exarrow-rs` → `0.12.7` (workspace dep `Cargo.toml:70`), `Cargo.lock` refreshed.
CI loses the unused `Checkout exarrow-rs` step + `build-contexts` line
(`.github/workflows/ci.yml:218-222,235`) since `Dockerfile.alpine` builds it
from the registry.

This is recorded as **ADR-029** at record time.

### Consequences

| Decision | Alternatives | Rationale |
|----------|--------------|-----------|
| External mode also uses `container_inner_ip()` | Keep `host:db_port`; add `EXASOL_CB_ADDRESS` env | The container is always `exasol-db` and exec-able in CI/local; loopback is the only failing address; simplest correct fix |
| Diagnostic on throwaway connection | Reconnect main `conn` after the diagnostic | Isolation is cleaner and removes the cascade that caused the misdiagnosis |
| Leave rowset codec alone | Audit/rewrite the scalar path | Evidence (shared Session ID, scenario-2-crashes-first) shows scalar path is collateral, not cause; verified by local repro |
| Bump exarrow-rs to 0.12.7 | Stay on 0.12.6 | User directive: latest crates.io release |

## Features

| Feature | Status | Spec |
|---------|--------|------|
| integration/connect-back | CHANGED | `specs/_plans/fix-it-matrix-connect-back-address/integration/connect-back/spec.md` |
| integration/db-roundtrip | CHANGED | `specs/_plans/fix-it-matrix-connect-back-address/integration/db-roundtrip/spec.md` |

## Dependencies

- `exasol/docker-db` images `2025.1.11`, `2025.2.1`, `2026.1.0`.
- `exapump`, a privileged Docker daemon, `rust:1.91-bookworm` (for glibc-2.36 `.so`).
- `exarrow-rs 0.12.7` from crates.io.

## Implementation Tasks

### Group A — IT harness fix (crates/it)
1. `crates/it/src/lib.rs`: rewrite `connect_back_sql_address()` (269–276) to
   always return `<container_inner_ip()>:8563`; never loopback; error if IP
   resolution fails. Update the doc comment (253–268) to drop the wrong
   external-mode branch.
2. `crates/it/tests/db_roundtrip.rs`: run `connect_back_python3_queries_and_emits`
   (call site 44–47) on a dedicated `harness.connect()` connection, closed after,
   so a VM crash never touches the shared `conn`.

### Group B — deps + CI (disjoint files)
3. `Cargo.toml:70`: `exarrow-rs` `0.12.5` → `0.12.7`; run
   `cargo +1.91 update -p exarrow-rs`; confirm `cargo +1.91 build` is clean.
4. `.github/workflows/ci.yml`: delete the `Checkout exarrow-rs` step (218–222)
   and the `build-contexts: exarrow-rs=...` line (235).

### Group C — docs/branding (disjoint files)
5. `README.md`: delete the `![Status: Alpha]` badge (line 6); rename prose
   "Script Language Container" → "Language Container" (lines 11, 19).
6. Rename spelled-out prose "Script Language Container" → "Language Container" in
   `specs/mission.md`, `specs/design.md` (and any other non-`_recorded` prose).
   KEEP: the `SLC` abbreviation, "SLC protocol", SQL `SCRIPT_LANGUAGES`/`CREATE
   SCRIPT`, `slc-rs`/`slc-rs-slim:dev`/`/opt/slc-rs` identifiers, glossary lines
   that *define* the acronym, and `specs/_recorded/`.

### Group D — spec deltas (authored under this plan dir; merged by /speq:record)
7. `integration/connect-back/spec.md`: rewrite the Background + the four
   `CB_SELF TO '<connect_back_sql_address()>'` parentheticals so both modes use
   `<container-eth0-ip>:8563`; state loopback/CoreDB-proxy is forbidden.
8. `integration/db-roundtrip/spec.md`: add Background + a scenario documenting
   that the non-fatal Python3 connect-back diagnostic runs on an isolated
   throwaway connection so it cannot poison the shared session used by the
   asserted scenarios.

## Verification

### Checklist (automated)
- `cargo +1.91 build` — exit 0.
- `cargo +1.91 fmt --check` / `clippy` per repo config — clean.
- `speq feature validate integration/connect-back` and `.../db-roundtrip` — pass
  (run against the merged specs after /speq:record; delta files must be valid).

### Scenario coverage
- Every `integration/connect-back` and `integration/db-roundtrip` scenario has a
  passing test in the local external-mode run below.

### Manual testing — PROOF = green ITs locally, external mode (must run BEFORE push)

> glibc trap: build the UDF `.so` inside `rust:1.91-bookworm` (glibc 2.36,
> matching the SLC). This host is Debian 13 (glibc ~2.41); host-built `.so`
> would require GLIBC_2.41 and fail to `dlopen` in the SLC.

1. (Reproduce first) With current code, run external mode against a Docker
   `exasol-db` and confirm `python3_connect_back` crashes the VM and `double_it`
   then fails on the same Session ID.
2. Apply Groups A–C.
3. Build `.so` in bookworm → `target/release/`:
   `docker run --rm -v "$PWD":/build -w /build rust:1.91-bookworm bash -c
   "apt-get update && apt-get install -y protobuf-compiler && cargo build
   --release -p scalar-double -p set-filter -p json-parse -p single-call-fixture
   -p connect-back-cluster-ip -p connect-back-query -p connect-back-insert
   -p connect-back-crunch"`.
4. `docker build -f Dockerfile.alpine -t slc-rs-slim:dev .`.
5. Start `exasol-db` (`--privileged --shm-size=2g --memory=6g
   -e COSLWD_ENABLED=1 -p 8563:8563 -p 2581:2581 exasol/docker-db:2026.1.0`);
   wait for "All stages finished."; read BucketFS write password from EXAConf.
6. Recompile the IT binary with the fix:
   `cargo +1.91 test -p it --features integration,db-2026-1 --no-run`; copy to
   `./it-runner`.
7. `EXASOL_HOST=localhost EXASOL_PORT=8563 BUCKETFS_PORT=2581
   BUCKETFS_PASSWORD=<pw> ./it-runner` → **all scenarios pass** (double_it +
   connect-back included).
8. (Ideal) repeat for `2025.1.11` and `2025.2.1`.
9. Push branch; confirm GitHub `integration` matrix is green.
