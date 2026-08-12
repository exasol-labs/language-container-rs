# Plan: fix-install-personal-local-db-port

## Summary

`scripts/install.sh`'s local Personal branch hardcodes the SQL port to `8563`, so on a host with several local deployments it registers `SCRIPT_LANGUAGES` against whichever database answers `127.0.0.1:8563` while reporting success (issue #83). Resolve the port from the descriptor's `connection.dbPort` through a testable `resolve_local_connection`, honoring `--port` the way the cloud branch already does.

## Design

### Context

The local branch inlines its connection decision in `main` (`scripts/install.sh:400-401`): `HOST="$PERSONAL_DB_HOST"` then `PORT="$PERSONAL_DB_PORT"`, where `PERSONAL_DB_PORT=8563` is a file-level constant (`scripts/install.sh:41`). The port is not a property of local Personal: the launcher forwards each deployment's SQL endpoint to `127.0.0.1` on a per-deployment port (`exasol config set --ports db:<port>`, launcher 2.2.0) and records it in `deployment.json` as `.connection.dbPort`. With three deployments (`default`:8563, `agent-alpha`:52164, `agent-beta`:59446), `--deployment agent-alpha` copies the SLC into agent-alpha's VM correctly, then issues `ALTER SYSTEM SET SCRIPT_LANGUAGES` against `default` — corrupting one deployment's working registration and leaving the target's untouched, with a success banner either way. Line 401 also overwrites the `PORT` the arg loop set at line 349, so `--port` is silently discarded.

Two forces make this a design decision rather than a one-line edit. First, the same descriptor field already has a correct, tested reader on the cloud side: `resolve_cloud_connection` (`scripts/install.sh:232-265`) takes it from the shared `deployment_field` helper with an `8563` default and lets `CLI_PORT` win. Second, the local branch's connection logic is unreachable from `scripts/tests/install-personal-test.sh`, which sources `install.sh` and calls functions in isolation — it can never run `main`'s body. That gap is why the defect shipped: every cloud connection value is pinned by a unit assertion, and no local one is.

- **Goals** — Resolve the local SQL port from `connection.dbPort` (default `8563`) on every run; honor `--port` for local as cloud already does; give the local host/port decision one owner that the sourced harness can drive in-process.
- **Non-Goals** — No change to the local branch's password resolution, forced `SCOPE=SYSTEM`, SSH port/node-key reads, SSH transport, or entry preservation; no change to the cloud branch or the non-`--deployment` HTTP path; no `--host` override for local (the forwarder is always `127.0.0.1`); no numeric validation of `connection.dbPort` beyond what cloud does today. Two follow-ups from issue #83 are deliberately deferred and are NOT oversights: printing the resolved `host:port` in the "Registering" banner (`scripts/install.sh:476`), and folding `deployment_ssh_port` into `deployment_field`. Both are candidates for their own issues.

### Decision

Add `resolve_local_connection <dir>` next to `resolve_cloud_connection`, make it the single owner of the local host/port decision, and call it from the local branch in place of the two hardcoding lines.

#### Architecture

```
install.sh --deployment NAME
        │
        ▼
  deployment_backend(dir)
        │
        ├─ "local" ──▶ resolve_local_connection(dir)          ← NEW
        │                HOST = 127.0.0.1            (forwarder, fixed)
        │                PORT = CLI_PORT ?: deployment_field .connection.dbPort ?: 8563
        │              then (UNCHANGED, stays in main):
        │                SCOPE=SYSTEM, secrets.json password, ssh port, node key
        │                                    │
        │                                    ▼ step 3: SSH copy + ALTER SYSTEM over $PORT
        │
        └─ other ────▶ resolve_cloud_connection(dir)          (UNCHANGED)
                         HOST/PORT/USER/PASSWORD ← descriptor, CLI flags win
```

`resolve_local_connection` mirrors the established seam exactly: it reads through `deployment_field`, assigns the globals `HOST` and `PORT`, and `return`s 1 (never `exit`s) so the sourced harness drives it under `set +e`. `main` calls it in a condition context — `resolve_local_connection "$DEPLOYMENT_DIR" || die …` — which suspends `errexit` inside the function. The `8563` fallback moves out of an inline assignment into the argument of that call, and the constant carrying it is renamed `PERSONAL_DB_PORT_DEFAULT` so its name states "fallback", not "fact".

#### Patterns

| Pattern | Where | Why |
|---------|-------|-----|
| Sourced-function seam (`return`, never `exit`) | `resolve_local_connection` | The harness cannot run `main`; a function that assigns globals and returns a status is the only local-branch logic it can assert against |
| One reader for every `.connection.*` field | `resolve_local_connection` → `deployment_field` | `deployment_field` is documented as the sole reader; local now uses it, so `dbPort` has one parsing behavior across both backends |
| CLI-override sentinel (`CLI_PORT`) | `resolve_local_connection` | Reuses the arg loop's existing capture (`scripts/install.sh:349`); no new flag, no new global, identical precedence to cloud |
| Constant named for its role, not its value | `PERSONAL_DB_PORT_DEFAULT` | `PERSONAL_DB_PORT` read as an assertion about Personal, which is what invited the hardcode |

#### Interface justification (`/speq:design-philosophy` Quick Diagnostic)

`resolve_local_connection` is a short function, so its depth is stated honestly: it does not hide volume, it owns a decision. One sentence covers its responsibility — "where a local deployment's SQL endpoint is". Nothing outside it needs to know how it is answered: `main` and step 3 read only `HOST` and `PORT`, so changing the source of the port (a new descriptor field, a launcher API) is an edit inside the function alone. Exactly one module now owns that decision, where today it is split between a file-level constant and two lines of `main`. Its value over the inline form is the test seam and that single ownership — accepted deliberately, because the untestable inline form is what let the defect ship.

### Consequences

| Decision | Alternatives Considered | Rationale |
|----------|------------------------|-----------|
| Extract `resolve_local_connection`, mirroring `resolve_cloud_connection` | Fix in place: `PORT="$(deployment_field "$DEPLOYMENT_DIR" '.connection.dbPort' 8563)"` inside `main` | The inline form is correct but stays unreachable from the sourced harness — the precise reason this bug went unnoticed while every cloud value was pinned |
| Extract host/port only; password, `SCOPE`, SSH port, node key stay in `main` | Extract the whole local branch, fully mirroring cloud's ownership (password + presence checks) | Keeps the fix's diff to the defect: moving `die` calls and unrelated reads is a refactor with its own review surface, deferred to a follow-up |
| Honor `--port` for local | Keep ignoring it (current documented behavior); reject `--port` with an error for local | Silently discarding an explicit flag is the same defect class as the hardcode; cloud already honors it, so symmetry removes a per-backend rule |
| Read `dbPort` via `deployment_field`, no numeric validation | Validate numerically like `deployment_ssh_port` does | Cloud reads the same field with no validation; validating on one side only would give one descriptor field two behaviors. A malformed value surfaces as an `exapump` connection error |
| Keep `HOST` fixed at `127.0.0.1`, stated in `usage()` | Honor `--host` for local; read `.connection.host` for local | The local SQL endpoint is always a forwarder on the invoking host, so an override is meaningless; documenting the ignore removes the silence without adding behavior |
| Rename `PERSONAL_DB_PORT` → `PERSONAL_DB_PORT_DEFAULT` | Leave the name; drop the constant and pass a literal `8563` | The misleading name is what the hardcode leaned on; the constant is still worth having, because `usage()` interpolates the same default |

## Features

| Feature | Status | Spec |
|---------|--------|------|
| container/personal-install | CHANGED | `container/personal-install/spec.md` |

## Impact

Operators running one local Personal deployment on `8563` see no change: an absent `connection.dbPort` still resolves to `8563`. Operators running several local deployments get the fix — `scripts/install.sh --deployment <name>` now registers against the database that `<name>`'s descriptor names, instead of whatever answers `127.0.0.1:8563`. One additional behavior change: `--port` is now honored for a local `--deployment` (previously discarded without warning), so `--help` and `docs/installation.md` no longer claim it is ignored. `--host` remains fixed at `127.0.0.1` for local, now stated in `usage()`. No flag is added or removed, and no non-`--deployment` invocation changes.

Anyone whose registration was silently landing on the wrong database should re-run the install for every affected deployment: the corrupted deployment keeps a `RUST=` entry pointing at a bucket path that its own BucketFS does not carry.

## Dependencies

None. `jq` is already required by the `--deployment` path, and `deployment_field` already exists.

## Implementation Tasks

1. Add three assertions to `scripts/tests/install-personal-test.sh` and register them in the `run` list (after `reads_ssh_port_from_deployment_json`), modeled on `resolves_cloud_connection_from_descriptor` / `cli_flags_override_cloud_descriptor` / `resolves_cloud_defaults_when_connection_fields_absent`. Each builds a `mktemp -d` fixture, calls `reset_connection_globals`, sets the sentinel `PORT="unresolved"`, then runs `resolve_local_connection "$dir"` and asserts via `check`. All three tests MUST set that sentinel, re-setting it before every `resolve_local_connection` call. Reason: `reset_connection_globals` leaves `PORT=8563` (`scripts/tests/install-personal-test.sh:126`), so without the sentinel an `8563` assertion passes even when the function never assigns `PORT`. That is the vacuous shape of the model test `resolves_cloud_defaults_when_connection_fields_absent` (`:251-269`) — copy its structure, not its unguarded assertion. The sentinel cannot leak, because every later test that reads `PORT` calls `reset_connection_globals` first. Use the ports from the issue's traced reproduction, never `8563`, for the descriptor-derived cases. (a) `resolves_local_connection_from_descriptor`: descriptor `{"backend":"local","connection":{"host":"127.0.0.1","sshPort":52341,"dbPort":52164}}` asserts return `0`, `HOST` is `127.0.0.1` and `PORT` is `52164`; then re-set the sentinel, rewrite the same descriptor with `"dbPort":59446`, resolve again, and assert `PORT` is `59446` (never cached, mirroring the second read in `reads_ssh_port_from_deployment_json`); then `rm -f "$dir/deployment.json"` and assert the resolution returns `1`. (b) `cli_port_overrides_local_descriptor`: same first descriptor, sentinel set, `CLI_PORT="1234"` pre-set exactly as `main`'s arg loop would, assert `PORT` is `1234`. (c) `resolves_local_defaults_when_db_port_absent`: descriptor `{"backend":"local","connection":{"host":"127.0.0.1","sshPort":52341}}`, sentinel set, assert return `0` and `PORT` is `8563`. Here the sentinel is what makes the assertion falsifiable: it proves `resolve_local_connection` applied the `deployment_field` default instead of leaving the `8563` that `reset_connection_globals` wrote. Each fixture directory is removed with `rm -rf "$dir"` at the end of its function. These fail until task 2 lands (`resolve_local_connection: command not found`).
2. Add `resolve_local_connection` to `scripts/install.sh` immediately after `resolve_cloud_connection` (`scripts/install.sh:265`), and rename `PERSONAL_DB_PORT` (`scripts/install.sh:41`) to `PERSONAL_DB_PORT_DEFAULT`. The function takes the deployment directory, declares `local dir="$1" descriptor_port` on one line, then assigns `descriptor_port="$(deployment_field "$dir" '.connection.dbPort' "$PERSONAL_DB_PORT_DEFAULT")" || return 1` on its own line — the split matters for the same reason the comment at `scripts/install.sh:237-238` records: a combined `local x="$(…)"` masks the command substitution's status behind the `local` builtin's. It then sets `HOST="$PERSONAL_DB_HOST"`, sets `PORT` to `$CLI_PORT` when that is non-empty and to `$descriptor_port` otherwise, and returns `0`. Give it a doc comment stating the design intent: the local SQL endpoint is a launcher-managed forwarder on `127.0.0.1` whose port is assigned per deployment and recorded as `connection.dbPort`, so a hardcoded `8563` registers against another deployment's database; the function is the single owner of that decision and returns (never exits) so the sourced harness drives it. In `main`'s local branch, replace `HOST="$PERSONAL_DB_HOST"` and `PORT="$PERSONAL_DB_PORT"` (`scripts/install.sh:400-401`) with `resolve_local_connection "$DEPLOYMENT_DIR" || die "cannot resolve the local deployment connection from $DEPLOYMENT_DIR"`. Leave `LOCAL_TRANSPORT=1`, `SCOPE=SYSTEM`, the `secrets.json` password resolution and its `die`, the `deployment_ssh_port` read, and the node-key check (`scripts/install.sh:399, 402-410`) untouched.
3. Correct the port prose in `scripts/install.sh`: the local bullet in the file-header comment (`scripts/install.sh:13-19`) — it explains why the SSH port is read fresh and must say the same of the SQL port; the `-D, --deployment` local block in `usage()` (`scripts/install.sh:90-94`), which claims host/port are fixed at `${PERSONAL_DB_HOST}:${PERSONAL_DB_PORT}` — state that the host is fixed at `${PERSONAL_DB_HOST}` (so `--host` is ignored) and the port comes from the descriptor's `.connection.dbPort`, defaulting to `${PERSONAL_DB_PORT_DEFAULT}`; the `-P, --port` line (`scripts/install.sh:104`), whose "ignored for local `--deployment`" is now false — `--port` overrides the resolved port on both backends; and the transport-selection comment's parenthetical `(forced 127.0.0.1:8563, ALTER SYSTEM)` (`scripts/install.sh:375-377`). Keep the two lines that legitimately name `8563` as a default: the `PORT=8563` global (`scripts/install.sh:48`) and the cloud `deployment_field` default (`scripts/install.sh:240`).
4. Correct the same claim in `docs/installation.md` § Exasol Personal install: "publishes only the SQL port (`8563`) from its VM" (line 57) and "it fixes the SQL host/port at `127.0.0.1:8563`" (line 81) both assert a fixed port — say instead that the host is `127.0.0.1` and the port comes from the deployment's `connection.dbPort` (`8563` when absent), with `--port` as an override; extend the step table's "Read the connection details" row (line 118) to list the SQL port alongside `connection.sshPort` and the node key; and change the "Register" row's "over `8563`" (line 120) to the resolved SQL port. Add one sentence noting that a host running several local deployments serves each on its own port, so the descriptor is what decides which database gets the registration.

No task is tagged `[expert]`: each is a localized shell or prose edit that copies a pattern already present and commented in the same file.

## Parallelization

| Parallel Group | Tasks |
|----------------|-------|
| Group A | Task 1 |
| Group B | Task 2 |
| Group C | Task 3, Task 4 |

Sequential dependencies:
- Group A → Group B (the assertions define the function's contract and fail until it exists)
- Group B → Group C (the prose describes the shipped behavior, and task 3 references the renamed constant)

## Dead Code Removal

| Type | Location | Reason |
|------|----------|--------|
| Constant | `scripts/install.sh:41` `PERSONAL_DB_PORT` | Renamed to `PERSONAL_DB_PORT_DEFAULT`, not deleted: it remains the fallback passed to `deployment_field` and the value `usage()` interpolates. No reference survives outside `install.sh` (verified: the only uses are lines 91 and 401) |

No code becomes dead. The two hardcoding lines are replaced, not stranded.

## Verification

### Scenario Coverage

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| Connection details are read fresh on every run | Unit + Manual | `scripts/tests/install-personal-test.sh` (SSH port: existing; SQL port: the second read in the new descriptor test); live re-run after `exasol stop`/`start` | `reads_ssh_port_from_deployment_json`, `resolves_local_connection_from_descriptor` |
| Local connection details resolve from the deployment directory | Unit | `scripts/tests/install-personal-test.sh` | `resolves_local_connection_from_descriptor`, `cli_port_overrides_local_descriptor`, `resolves_local_defaults_when_db_port_absent` |
| A registered Rust UDF executes on Personal | Manual | Live local Personal deployment on a non-`8563` port | See Manual Testing |

The local scenario's default-fallback clause (`connection.dbPort` absent resolves to `8563`) counts as covered only because task 1(c) sets the `PORT="unresolved"` sentinel before resolving. Drop the sentinel and `reset_connection_globals`' own `PORT=8563` satisfies the assertion, making this row's coverage claim false.

The remaining nine scenarios are untouched by this plan and stay covered as recorded: `fragment_points_at_executable_no_leading_slash`, `preserves_existing_script_languages`, `parses_current_script_languages_from_query_output`, `selects_transport_from_backend`, the five `cloud_*`/`resolves_cloud_*` tests, plus manual end-to-end verification on live local and cloud deployments.

Residual coverage gap, unchanged by this plan: the arg-loop capture of `--port` into `CLI_PORT` (`scripts/install.sh:349`) cannot be unit-tested, because the sourced harness never runs `main`'s loop. The unit tests cover `resolve_local_connection`'s consumption of a pre-set `CLI_PORT`; the loop itself is exercised by the `--port` manual run below.

### Manual Testing

Requires two live local Personal deployments on different SQL ports, e.g. `exasol config set --ports db:52164` for `agent-alpha` while `default` keeps `8563`.

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| container/personal-install (local, non-default port) | `SLC_TARBALL=lc-rs.tar.gz scripts/install.sh --deployment agent-alpha --skip-build` | Succeeds and prints the `RUST=…` entry; `exapump sql "SELECT SYSTEM_VALUE FROM EXA_PARAMETERS WHERE PARAMETER_NAME = 'SCRIPT_LANGUAGES'" -d "exasol://sys:<pw>@127.0.0.1:52164?validateservercertificate=0"` shows the `RUST` entry on agent-alpha |
| container/personal-install (local, other deployment untouched) | Same query against `127.0.0.1:8563` after the run above | `default`'s `SCRIPT_LANGUAGES` is unchanged — no `RUST` entry added or overwritten there |
| container/personal-install (local, `--port` honored) | `SLC_TARBALL=lc-rs.tar.gz scripts/install.sh --deployment agent-alpha --port 9999 --skip-build` | Fails at the registration step with a connection error against `127.0.0.1:9999`, proving the flag reaches the DSN instead of being discarded |
| container/personal-install (local, default preserved) | `SLC_TARBALL=lc-rs.tar.gz scripts/install.sh --deployment default --skip-build` on a descriptor with no `connection.dbPort` | Registers over `8563` exactly as before |
| container/personal-install (UDF executes) | `CREATE OR REPLACE RUST SCALAR SCRIPT` + `SELECT` over `127.0.0.1:52164` | Returns the expected result from the SLC installed on agent-alpha |

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Install-script tests | `bash scripts/tests/install-personal-test.sh` | `All assertions passed.` (exit 0) |
| Shellcheck | `shellcheck scripts/install.sh scripts/tests/install-personal-test.sh` | 0 warnings |
| Build | `cargo build --release` | Exit 0 |
| Test | `cargo test` | 0 failures |
| Lint | `cargo clippy --all-targets --all-features -- -D warnings` | 0 warnings |
| Format | `cargo fmt --check` | No changes |
