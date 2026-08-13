# Plan: fix-install-personal-local-db-port

## Summary

`scripts/install.sh`'s local Personal branch hardcoded the SQL port to `8563`, so on a host with several local deployments it registered `SCRIPT_LANGUAGES` against whichever database answered `127.0.0.1:8563` while reporting success (issue #83). Resolve every Personal connection field — host, port, user, DB password — through one `resolve_deployment_connection` shared by both backends, with command-line flags overriding, so no per-backend rule can bake a launcher implementation detail into the installer again.

## Design

### Context

The defect was a file-level constant read as a fact: `PERSONAL_DB_PORT=8563` assigned straight into `main`'s local branch. The first pass (tasks 1-4, shipped) replaced it with `resolve_local_connection`, which read `.connection.dbPort` through the shared `deployment_field` helper, honored `--port`, and gave the sourced test harness a seam it could drive. That fix is verified: `scripts/tests/install-personal-test.sh` pins it, and a stubbed end-to-end run confirmed `dbPort: 52164` → DSN `exasol://sys:pw@127.0.0.1:52164`.

The PR review (#85) then rejected what the fix was about to record. `resolve_local_connection` kept `HOST="$PERSONAL_DB_HOST"` unconditionally and the plan documented that as permanent behavior — "the host is fixed at `127.0.0.1`, `--host` is ignored". That claim is the same class of assumption that produced #83: it bakes the launcher's current forwarder design into the installer. If Personal ever assigns per-deployment addresses (a loopback alias, or a VM IP instead of a forwarder), the local branch silently registers against whatever answers `127.0.0.1`, `--host` is discarded, and the operator has no workaround short of editing the script. Reading `.connection.host` with a default costs one `deployment_field` call on a seam that already exists and is already tested.

The two resolvers also diverged for no reason a reader could name. `resolve_cloud_connection` owned four fields, the DB-password presence check, and the `--bfs-password` check; `resolve_local_connection` owned two, while `main`'s local branch resolved the password inline with its own `die`. Same descriptor, same helper, two precedence rules to keep in sync.

- **Goals** — Resolve host, port, user, and DB password from the deployment directory through a single backend-agnostic function on both paths, with `--host`/`--port`/`--user`/`--password` overriding; leave the backend's host default as the only per-backend connection difference; make a wrong target visible by printing the resolved endpoint before registering; warn when a local descriptor omits `.connection.dbPort`, because the silent `8563` fallback is the original bug.
- **Non-Goals** — No change to cloud behavior; no change to the local branch's forced `SCOPE=SYSTEM`, SSH port read, node-key check, SSH transport, or entry preservation; no `--bfs-password` requirement for local; no numeric validation of `.connection.dbPort`; no folding of `deployment_ssh_port` into `deployment_field` (its numeric check is a behavior question of its own). `--host` retargets the SQL endpoint only: the SSH copy keeps `SSH_HOST=127.0.0.1`, because the descriptor names no SSH host and inventing one is a separate change.

### Decision

Delete `resolve_cloud_connection` and `resolve_local_connection`. Add `resolve_deployment_connection <dir> <default_host>`, which sets `HOST`, `PORT`, `USER`, and `PASSWORD` under one precedence table. Both call sites pass their backend's host default; everything genuinely per-backend stays at the call site.

| Field | Precedence |
|-------|------------|
| host | `--host` → `.connection.host` → `<default_host>` → fail when empty |
| port | `--port` → `.connection.dbPort` → `8563` |
| user | `--user` → `.connection.username` → `sys` |
| password | `--password` → `secrets.json` `.dbPassword` → fail when empty |

#### Architecture

```
install.sh --deployment NAME
        │
        ▼
  deployment_backend(dir)
        │
        ├─ "local" ──▶ resolve_deployment_connection(dir, 127.0.0.1)
        │              then, at the call site (local-only):
        │                SCOPE=SYSTEM, missing-dbPort warning,
        │                ssh port, node key
        │                                    │
        │                                    ▼ step 3: SSH copy + ALTER SYSTEM
        │                                       banner prints ${HOST}:${PORT}
        │
        └─ other ────▶ resolve_deployment_connection(dir, '')
                       then, at the call site (cloud-only):
                         require_cloud_bfs_password
                                             │
                                             ▼ step 3: HTTP upload + ALTER <scope>
                                                banner prints ${HOST}:${PORT}
```

The function keeps `resolve_cloud_connection`'s established seam: it reads through `deployment_field`, captures each accessor on its own line so the `local` builtin cannot mask a command substitution's exit status, assigns globals, and `return`s (never `exit`s) so the sourced harness drives it under `set +e`. `main` calls it in a condition context — `resolve_deployment_connection … || die …` — which suspends `errexit` inside, so a failed accessor falls through to the presence checks instead of aborting.

An unreadable `deployment.json` still fails: `deployment_field` returns 1 and prints nothing, so `HOST` stays empty and the host check fires. Reaching the resolver with an unreadable descriptor is possible only from the harness, because `main` calls `deployment_backend` first and dies there.

#### Patterns

| Pattern | Where | Why |
|---------|-------|-----|
| One owner for the precedence decision | `resolve_deployment_connection` | Two resolvers meant one rule written twice; the second copy is where a backend-specific assumption hides |
| Per-backend difference expressed as an argument, not a branch | `<default_host>` | The host default is the whole local/cloud connection difference, so it is the whole parameter list difference |
| Backend rules stay at the call site | `SCOPE=SYSTEM`, dbPort warning, `require_cloud_bfs_password` | A resolver that has to ask which backend it serves is not backend-agnostic; a `local`-only warning inside it would reintroduce the branch |
| Sourced-function seam (`return`, never `exit`) | `resolve_deployment_connection`, `require_cloud_bfs_password` | The harness cannot run `main`; only global-assigning functions that return a status are assertable |
| Constant named for its role | `PERSONAL_DB_HOST_DEFAULT` | `PERSONAL_DB_HOST` read as an assertion about Personal, matching `PERSONAL_DB_PORT_DEFAULT`, whose old name invited the hardcode |

#### Interface justification (`/speq:design-philosophy` Quick Diagnostic)

One sentence covers the responsibility: "where a Personal deployment's database is, and who logs in". The interface is one directory plus one default, against internals that read three descriptor fields, two files, two CLI sentinels, and two presence checks — so calling it is cheaper than restating it, and the previous duplication is the evidence. Nothing outside it needs to know how an answer was reached: callers read only `HOST`/`PORT`/`USER`/`PASSWORD`, so a new descriptor field or a launcher API change is an edit inside the function.

The dependency direction is unchanged and correct: the resolver names `deployment_field` and `deployment_db_password`, never a transport. It knows no backend name — the one place a backend leaked in (the `--bfs-password` check, which belongs to BucketFS rather than to a connection) moves out to `require_cloud_bfs_password` at the cloud call site.

Two decisions deliberately stay outside: the missing-`dbPort` warning is local-only policy, and the BucketFS credential is not a connection field. Putting either inside would force the function to ask which backend called it, which is the divergence this change removes.

### Consequences

| Decision | Alternatives Considered | Rationale |
|----------|------------------------|-----------|
| One `resolve_deployment_connection` for both backends | Keep both resolvers and fold `.connection.host` into the local one; defer unification to a follow-up | Two copies of one precedence rule is where the next backend-specific assumption hides, and the plan is unrecorded, so unification costs one review instead of two |
| `--host` overrides for local; `.connection.host` read with a `127.0.0.1` default | Keep the host pinned to `127.0.0.1` and document that `--host` is ignored (superseded) | Recording "fixed at `127.0.0.1`" as permanent behavior repeats #83's mistake: an operator whose deployment moves has no workaround but editing the script |
| Local user resolves from `.connection.username`, default `sys` | Keep `sys` for local | Symmetry costs one accessor already in the function; real descriptors say `sys`, so today's behavior is unchanged |
| `require_cloud_bfs_password` as its own function at the cloud call site | Keep the check inside the resolver, skipped when `<default_host>` is non-empty; inline it in `main`'s cloud branch | A BucketFS credential is not a connection field, and gating it on the host default would smuggle the backend back in. Its own function keeps the check assertable, which inlining in `main` would lose |
| Warn from the local call site by re-reading `.connection.dbPort` | Export a "port came from the descriptor" flag from the resolver; skip the warning | A second `jq` read is cheaper than a global that leaks how resolution happened; without the warning a malformed descriptor still lands on another deployment's port |
| Print `${HOST}:${PORT}` in both registration banners | Local banner only, as the review's finding names | The cloud path resolves its endpoint from a descriptor the operator never sees, so the same blindness applies; both banners are one line each |
| Rename `PERSONAL_DB_HOST` → `PERSONAL_DB_HOST_DEFAULT` | Leave the name | It is now a default, not a fact, and the matching `PERSONAL_DB_PORT_DEFAULT` rename is what made the port fix legible |

## Features

| Feature | Status | Spec |
|---------|--------|------|
| container/personal-install | CHANGED | `container/personal-install/spec.md` |

## Impact

Operators running one local Personal deployment on `8563` see no change. Operators running several get the fix: `scripts/install.sh --deployment <name>` registers against the database that `<name>`'s descriptor names, instead of whatever answers `127.0.0.1:8563`. Three flags change behavior for a local `--deployment`: `--port` and `--host` are honored instead of discarded, and `--user` now falls back to `.connection.username` rather than always `sys` (no practical change today — real descriptors say `sys`). Every run now prints the resolved `host:port` before registering, and a local descriptor missing `.connection.dbPort` produces a warning naming the fallback port. Cloud behavior is unchanged. No flag is added or removed, and no non-`--deployment` invocation changes.

Anyone whose registration was silently landing on the wrong database should re-run the install for every affected deployment: the corrupted deployment keeps a `RUST=` entry pointing at a bucket path that its own BucketFS does not carry.

## Dependencies

None. `jq` is already required by the `--deployment` path, and `deployment_field` already exists.

## Implementation Tasks

Tasks 1-4 shipped the port fix and are complete (`tasks.md` phases 2-4). Tasks 5-11 are this revision, unifying the two resolvers before the plan is recorded.

1. *(done)* Add three local assertions to `scripts/tests/install-personal-test.sh`, each carrying the `PORT="unresolved"` sentinel that keeps the default-fallback assertion falsifiable.
2. *(done)* Add `resolve_local_connection` to `scripts/install.sh`, rename `PERSONAL_DB_PORT` to `PERSONAL_DB_PORT_DEFAULT`, wire it into `main`'s local branch.
3. *(done)* Correct the port prose in `scripts/install.sh`.
4. *(done)* Correct the port prose in `docs/installation.md` § Exasol Personal install.

5. Retarget the nine existing resolver tests in `scripts/tests/install-personal-test.sh`, before their targets exist (they fail with `resolve_deployment_connection: command not found` until task 7). Five cloud tests (`resolves_cloud_connection_from_descriptor:230`, `cli_flags_override_cloud_descriptor:252`, `cloud_requires_db_password:301`, `resolves_cloud_defaults_when_connection_fields_absent:320`, `cloud_leaves_scope_untouched:340`) swap `resolve_cloud_connection "$dir"` for `resolve_deployment_connection "$dir" ''` — the empty default host is what keeps cloud's "no host default" rule asserted. In `resolves_cloud_defaults_when_connection_fields_absent`, add `PORT="unresolved"` and `USER="unresolved"` after `reset_connection_globals` (`:328`): its `8563` and `sys` assertions (`:334-335`) currently pass even if the function never assigns either global, because `reset_connection_globals` already wrote both values (`:195-196`). That is round 2's carried-forward advisory, and this task is already editing the line above it. The three local tests (`resolves_local_connection_from_descriptor:105`, `cli_port_overrides_local_descriptor:137`, `resolves_local_defaults_when_db_port_absent:156`) swap `resolve_local_connection "$dir"` for `resolve_deployment_connection "$dir" "$PERSONAL_DB_HOST_DEFAULT"`, and each gains `printf '{"dbPassword":"secret"}\n' >"$dir/secrets.json"` beside its `deployment.json`: the unified function fails on an empty password, so without that fixture all three would resolve to `return 1` and the two `rc` assertions (`:118`, `:168`) would fail. Then rework `resolves_local_connection_from_descriptor`, whose `HOST` assertions now invert. Keep the descriptor host `descriptor.example`, drop both `HOST="override.example"` lines (`:113`, `:122`), and assert `HOST` is `descriptor.example` after each of the two resolves, with the check text `"HOST resolves from connection.host"`. The fixture host must stay `descriptor.example` rather than `127.0.0.1`, because a `127.0.0.1` fixture cannot distinguish the descriptor value from the default — the round-1 vacuity in a new place. Add `check "PASSWORD resolves from secrets.json .dbPassword" "secret" "$PASSWORD"` after the first resolve, which is the first assertion the local password path has ever had. Before the missing-descriptor resolve (`:130-132`), set `HOST=""` and explain why in a comment: with no `.connection.host` to read and no `--host` supplied, the empty-host check is what fails the resolution, so a leftover `HOST` from an earlier assertion in the same function would make it return `0`. Finally retarget the ninth test, `cloud_requires_operator_bfs_password:282`, at `require_cloud_bfs_password` (task 7): keep `reset_connection_globals`, the `err=`/`rc=` capture, and both assertions, and drop the `mktemp -d` fixture with its `rm -rf`, because the function reads only `BFS_PASSWORD`. [expert]
6. Add the three new local tests from the review, registered in the `run` list (`:368-370`) beside the retargeted local ones. Each builds a `mktemp -d` fixture with `deployment.json` and `secrets.json`, calls `reset_connection_globals`, resolves with `"$PERSONAL_DB_HOST_DEFAULT"` as the default host, asserts through `check`, and ends with `rm -rf "$dir"`. (a) `cli_host_overrides_local_descriptor`: descriptor `{"backend":"local","connection":{"host":"descriptor.example","sshPort":52341,"dbPort":52164}}`, `HOST="override.example"` pre-set exactly as `main`'s arg loop would, asserts `HOST` is `override.example`. Overriding the descriptor value (not the default) is deliberate: the descriptor already beats the default, so this pins the top of the precedence chain. (b) `resolves_local_host_default_when_absent`: descriptor `{"backend":"local","connection":{"sshPort":52341,"dbPort":52164}}` with no `.connection.host`, asserts return `0` and `HOST` is `127.0.0.1`. No sentinel is possible or needed here — `HOST=""` is how "no `--host`" is expressed, and `reset_connection_globals` leaves exactly that, so an unassigned `HOST` fails the assertion. (c) `resolves_local_user_from_descriptor`: descriptor `{"backend":"local","connection":{"host":"127.0.0.1","sshPort":52341,"dbPort":52164,"username":"dbadmin"}}`, asserts `USER` is `dbadmin`. The username MUST NOT be `sys`: `reset_connection_globals` writes `USER=sys` (`:196`), so a `sys` fixture would assert nothing.
7. Replace both resolvers in `scripts/install.sh` with `resolve_deployment_connection <dir> <default_host>` (`:226-290`), and rename `PERSONAL_DB_HOST` to `PERSONAL_DB_HOST_DEFAULT` at `:42` together with its only other reference in `usage()` (`:93`) — one task, because splitting them leaves an intermediate commit whose `--help` aborts under `set -u`, the carried-forward round-1 advisory. Declare `local dir="$1" default_host="$2"`, then the descriptor variables, then capture each accessor on its own line with `|| true`, keeping `resolve_cloud_connection`'s comment explaining that a combined `local x="$(…)"` masks the command substitution's status: `.connection.host` defaulted to `$default_host`, `.connection.dbPort` to `$PERSONAL_DB_PORT_DEFAULT`, `.connection.username` to `sys`. Apply the precedence table exactly as `resolve_cloud_connection` does today — `[[ -z "$HOST" ]] && HOST="$descriptor_host"`, `CLI_PORT`/`CLI_USER` winning when non-empty, and the descriptor password read only when `PASSWORD` is empty — then the two presence checks, whose messages drop `for the cloud deployment` for `for the deployment` so they read correctly on both paths. Return `0`. Move the `--bfs-password` check into a new `require_cloud_bfs_password` beside it, keeping its error text verbatim so `cloud_requires_operator_bfs_password`'s stderr assertion holds, and give it a doc comment stating why a BucketFS credential is not a connection field. Write the resolver's doc comment as design intent: one precedence rule for both backends, the host default as the only per-backend difference, and returns-never-exits for the sourced harness. [expert]
8. Rewire both call sites in `main` (`scripts/install.sh:421-443`). Local: `resolve_deployment_connection "$DEPLOYMENT_DIR" "$PERSONAL_DB_HOST_DEFAULT" || die "cannot resolve the local deployment connection from $DEPLOYMENT_DIR"`, then delete the inline `secrets.json` password block and its `die` (`:432-433`), which the resolver now owns. Cloud: `resolve_deployment_connection "$DEPLOYMENT_DIR" '' || die "cannot resolve the cloud deployment connection from $DEPLOYMENT_DIR"` followed by `require_cloud_bfs_password || die "cannot install a cloud deployment without a BucketFS password"` — the specific error still comes from the function, the generic one from `main`, exactly as a resolver failure reads today. Leave `LOCAL_TRANSPORT=1`, `SCOPE=SYSTEM`, the `deployment_ssh_port` read, and the node-key check untouched. Add the missing-`dbPort` warning to the local branch after the resolve, guarded by `[[ -z "$CLI_PORT" ]]` because an explicit `--port` means no fallback happened: capture `descriptor_db_port="$(deployment_field "$DEPLOYMENT_DIR" '.connection.dbPort' '')" || true` on its own line, and when it is empty print to stderr that the descriptor carries no `.connection.dbPort` and that registering over the fallback `$PERSONAL_DB_PORT_DEFAULT` risks hitting another local deployment.
9. Print the resolved endpoint in both registration banners (`scripts/install.sh:502`, `:525`): `==> Registering RUST at ${HOST}:${PORT} (ALTER SYSTEM SET SCRIPT_LANGUAGES) …` and the `ALTER ${SCOPE_UPPER}` equivalent. State the endpoint before the existing parenthetical rather than nesting a second one. This is the deferred half of decision [8]; `decision-log.md` records that it would have made #83 self-diagnosing.
10. Correct the host prose in `scripts/install.sh`. The `-D, --deployment` local block in `usage()` (`:90-99`) claims the SQL host is fixed at `${PERSONAL_DB_HOST}` and that `--host` is ignored: say instead that host, port, user, and DB password are read from the deployment directory, that the host defaults to `${PERSONAL_DB_HOST_DEFAULT}` and the port to `${PERSONAL_DB_PORT_DEFAULT}`, and that `--host`, `--port`, `--user`, and `--password` override the resolved values. That block is where `--host` gets its Personal statement: `-H, --host` (`:86`) sits in the "Required (HTTP transport)" list, where "Exasol host" stays accurate and a `--deployment` clause would not belong. Leave `-P, --port` (`:109`) and `-u, --user` (`:110`) as they are — both already read as overrides on both backends. The transport-selection comment's parenthetical `(host fixed at 127.0.0.1, port resolved from connection.dbPort, ALTER SYSTEM)` (`:400-401`) becomes the SQL endpoint resolved from the descriptor with a `127.0.0.1` host default. Leave the file-header local bullet's two port sentences (`:12-21`) as they are — both are still accurate — and leave `SSH_HOST` (`:44`) alone, since `--host` retargets the SQL endpoint only.
11. Correct the same claim in `docs/installation.md` § Exasol Personal install: line 83's "it fixes the SQL host at `127.0.0.1`" becomes a `127.0.0.1` default that `.connection.host` and then `--host` override, and the paragraph names all four resolved fields with their overriding flags; soften line 57's "on `127.0.0.1`" to the same default. Extend the step table's "Read the connection details" row (line 124) to list `connection.host` and `connection.username` beside `connection.sshPort`, `connection.dbPort`, and the node key, and add to the "Register" row (line 126) that the resolved `host:port` is printed before the `ALTER`. Add one sentence stating that a local descriptor with no `connection.dbPort` is malformed and produces a warning naming the fallback port.

Tasks 5 and 7 are tagged `[expert]`: the first inverts live assertions where two of the three fixture values are the ones that make an assertion falsifiable, and the second rewrites a precedence body whose correctness depends on `local` not masking a command substitution's status and on `errexit` staying suspended in a condition context.

## Parallelization

| Parallel Group | Tasks |
|----------------|-------|
| Group D | Task 5 |
| Group E | Task 6 |
| Group F | Task 7 |
| Group G | Task 8 |
| Group H | Task 9, Task 11 |
| Group I | Task 10 |

Sequential dependencies:
- Group D → Group E (both edit `scripts/tests/install-personal-test.sh`, so they cannot run concurrently)
- Group E → Group F (the assertions define both functions' contracts and fail until they exist)
- Group F → Group G (the call sites need the resolver and `require_cloud_bfs_password`)
- Group G → Group H (the banners read the globals the call sites finalize, and the prose describes shipped behavior)
- Group H → Group I (tasks 9 and 10 both edit `scripts/install.sh`; task 11 touches only `docs/installation.md`, so it runs beside either)

Groups A-C (tasks 1-4) are complete.

## Dead Code Removal

| Type | Location | Reason |
|------|----------|--------|
| Function | `scripts/install.sh:226-270` `resolve_cloud_connection` and its doc comment | Superseded by `resolve_deployment_connection`; its call site and all six cloud tests move, so no reference survives |
| Function | `scripts/install.sh:272-290` `resolve_local_connection` and its doc comment | Same, for the local path: one call site and three tests |
| Inline block | `scripts/install.sh:432-433` local `secrets.json` password read and its `die` | The resolver owns the DB password on both paths; leaving this would apply the rule twice |
| Constant | `scripts/install.sh:42` `PERSONAL_DB_HOST` | Renamed to `PERSONAL_DB_HOST_DEFAULT`, not deleted: it is the local host default passed to the resolver and the value `usage()` interpolates |

`SSH_HOST` (`:44`) survives deliberately and is not dead: the SSH copy still targets the invoking host, which `--host` does not change.

## Verification

### Scenario Coverage

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| Local connection details resolve from the deployment directory | Unit | `scripts/tests/install-personal-test.sh` | `resolves_local_connection_from_descriptor`, `resolves_local_host_default_when_absent`, `resolves_local_user_from_descriptor`, `resolves_local_defaults_when_db_port_absent` |
| Command-line flags override descriptor-derived local values | Unit | `scripts/tests/install-personal-test.sh` | `cli_host_overrides_local_descriptor`, `cli_port_overrides_local_descriptor`, `cli_flags_override_cloud_descriptor` |
| A local descriptor that omits the SQL port is reported | Unit + Manual | `scripts/tests/install-personal-test.sh` (unreadable descriptor); live run on a descriptor with no `connection.dbPort` (warning) | `resolves_local_connection_from_descriptor` (third resolve); see Manual Testing |
| Registration is system-scoped and preserves existing entries | Unit + Manual | `scripts/tests/install-personal-test.sh` (entry preservation, existing); live run (printed endpoint) | `preserves_existing_script_languages`; see Manual Testing |
| Connection details are read fresh on every run | Unit + Manual | `scripts/tests/install-personal-test.sh` (SSH port: existing; SQL port: the second read in the descriptor test); live re-run after `exasol stop`/`start` | `reads_ssh_port_from_deployment_json`, `resolves_local_connection_from_descriptor` |
| Local install resolves the DB password from the deployment directory | Unit | `scripts/tests/install-personal-test.sh` | `resolves_local_connection_from_descriptor` (new `PASSWORD` assertion) |
| Cloud connection details resolve from the deployment directory | Unit | `scripts/tests/install-personal-test.sh` | `resolves_cloud_connection_from_descriptor`, `resolves_cloud_defaults_when_connection_fields_absent` |
| Command-line flags override descriptor-derived cloud values | Unit | `scripts/tests/install-personal-test.sh` | `cli_flags_override_cloud_descriptor` |
| Cloud install requires an operator-supplied BucketFS password | Unit | `scripts/tests/install-personal-test.sh` | `cloud_requires_operator_bfs_password` |
| Cloud install fails when no DB password resolves | Unit | `scripts/tests/install-personal-test.sh` | `cloud_requires_db_password` |
| Cloud install uses the standard HTTP transport and scope | Unit + Manual | `scripts/tests/install-personal-test.sh`; live cloud deployment | `cloud_leaves_scope_untouched`; see Manual Testing |
| A registered Rust UDF executes on Personal | Manual | Live local Personal deployment on a non-`8563` port | See Manual Testing |

Two local clauses are covered by assertions that are falsifiable only because of a chosen fixture value. The `.connection.dbPort` default needs `resolves_local_defaults_when_db_port_absent`'s `PORT="unresolved"` sentinel, because `reset_connection_globals` writes `PORT=8563` (`:195`). The `.connection.host` clause needs the fixture host `descriptor.example`, because a `127.0.0.1` fixture cannot tell the descriptor value from the default. Task 5 also keeps `resolves_cloud_defaults_when_connection_fields_absent`'s two cloud defaults from staying the last unguarded pair in the file.

The local override scenario's `--user` and `--password` clauses map to `cli_flags_override_cloud_descriptor`. After unification the four override branches are one code path with no backend test in them, so that test pins them for both backends; only the host default differs, and `cli_host_overrides_local_descriptor` covers it.

Three behaviors have manual coverage only, because they live in `main`, which the sourced harness cannot run: the missing-`dbPort` warning, the printed endpoint, and `main`'s calls to the resolver and `require_cloud_bfs_password`. The `--port`/`--user` arg-loop capture into `CLI_PORT`/`CLI_USER` is the same pre-existing gap, unchanged by this plan.

The remaining scenarios are untouched and stay covered as recorded: `fragment_points_at_executable_no_leading_slash`, `preserves_existing_script_languages`, `parses_current_script_languages_from_query_output`, `selects_transport_from_backend`, plus manual end-to-end verification on live local and cloud deployments.

### Manual Testing

Requires two live local Personal deployments on different SQL ports, e.g. `exasol config set --ports db:52164` for `agent-alpha` while `default` keeps `8563`.

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| container/personal-install (local, non-default port) | `SLC_TARBALL=lc-rs.tar.gz scripts/install.sh --deployment agent-alpha --skip-build` | Succeeds, prints `Registering RUST at 127.0.0.1:52164`, and prints the `RUST=…` entry; `exapump sql "SELECT SYSTEM_VALUE FROM EXA_PARAMETERS WHERE PARAMETER_NAME = 'SCRIPT_LANGUAGES'" -d "exasol://sys:<pw>@127.0.0.1:52164?validateservercertificate=0"` shows the `RUST` entry on agent-alpha |
| container/personal-install (local, other deployment untouched) | Same query against `127.0.0.1:8563` after the run above | `default`'s `SCRIPT_LANGUAGES` is unchanged — no `RUST` entry added or overwritten there |
| container/personal-install (local, `--port` honored) | `SLC_TARBALL=lc-rs.tar.gz scripts/install.sh --deployment agent-alpha --port 9999 --skip-build` | Fails at the registration step with a connection error against `127.0.0.1:9999`, and the banner reads `at 127.0.0.1:9999` |
| container/personal-install (local, `--host` honored) | `SLC_TARBALL=lc-rs.tar.gz scripts/install.sh --deployment agent-alpha --host 127.0.0.2 --skip-build` | The SSH copy still succeeds against `127.0.0.1`, then registration fails against `127.0.0.2:52164`, proving `--host` reaches the DSN and leaves the SSH transport alone |
| container/personal-install (local, malformed descriptor) | Remove `connection.dbPort` from `default`'s `deployment.json`, then `SLC_TARBALL=lc-rs.tar.gz scripts/install.sh --deployment default --skip-build` | Warns on stderr that the descriptor carries no `.connection.dbPort` and that registering over `8563` risks hitting another local deployment, then registers over `8563` exactly as before |
| container/personal-install (UDF executes) | `CREATE OR REPLACE RUST SCALAR SCRIPT` + `SELECT` over `127.0.0.1:52164` | Returns the expected result from the SLC installed on agent-alpha |
| container/personal-install (cloud, unchanged) | `SLC_TARBALL=lc-rs.tar.gz scripts/install.sh --deployment my-cloud-db --bfs-password <pw> --skip-build`, then the same run without `--bfs-password` | The first uploads and registers over the descriptor's host and port, with the banner naming them; the second fails naming `--bfs-password` |

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Install-script tests | `bash scripts/tests/install-personal-test.sh` | `All assertions passed.` (exit 0) |
| Shellcheck | `shellcheck scripts/install.sh scripts/tests/install-personal-test.sh` | No new warnings (baseline SC1091/SC2034 unchanged) |
| Build | `cargo build --release` | Exit 0 |
| Test | `cargo test` | 0 failures |
| Lint | `cargo clippy --all-targets --all-features -- -D warnings` | 0 warnings |
| Format | `cargo fmt --check` | No changes |
