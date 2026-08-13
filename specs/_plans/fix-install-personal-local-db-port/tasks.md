# Tasks: fix-install-personal-local-db-port

## Phase 2: Implementation (Group A)
- [x] 2.1 Add three assertions to `scripts/tests/install-personal-test.sh` (resolves_local_connection_from_descriptor, cli_port_overrides_local_descriptor, resolves_local_defaults_when_db_port_absent), register them in the `run` list

## Phase 2: Implementation (Group B)
- [x] 2.2 Add `resolve_local_connection` to `scripts/install.sh`, rename `PERSONAL_DB_PORT` to `PERSONAL_DB_PORT_DEFAULT`, wire into `main`'s local branch

## Phase 2: Implementation (Group C)
- [x] 2.3 Correct port prose in `scripts/install.sh` (header comment, usage(), --port line, transport-selection comment)
- [x] 2.4 Correct port prose in `docs/installation.md` § Exasol Personal install

## Phase 3: Verification
- [x] 3.1 Run `bash scripts/tests/install-personal-test.sh`
- [x] 3.2 Run `shellcheck scripts/install.sh scripts/tests/install-personal-test.sh`
- [x] 3.3 Run `cargo build --release`
- [x] 3.4 Run `cargo test`
- [x] 3.5 Run `cargo clippy --all-targets --all-features -- -D warnings`
- [x] 3.6 Run `cargo fmt --check`

## Phase 4: Review Fixes
- [x] 4.1 In `scripts/tests/install-personal-test.sh`, function `resolves_local_connection_from_descriptor`: change `.connection.host` in both descriptor `printf` payloads (lines 109 and 122) from `"127.0.0.1"` to `"descriptor.example"`; after `reset_connection_globals` (line 112) add `HOST="override.example"` alongside the existing `PORT="unresolved"` sentinel, and re-set `HOST="override.example"` next to the second `PORT="unresolved"` (line 121) before the reassigned-`dbPort` resolve; keep both `HOST` assertions expecting `127.0.0.1` and rename the line-118 check text to `"HOST is fixed at the local forwarder, overriding both --host and the descriptor"`; add the same `HOST` assertion after the second resolve. Do not change the descriptor host in `cli_port_overrides_local_descriptor` or `resolves_local_defaults_when_db_port_absent`.
- [x] 4.2 In `scripts/install.sh`, rewrite the trailing clause of the local bullet (lines 17-21) so the two ports have their own reasons: the SSH port is reassigned on every `exasol start`, so re-running after `exasol stop && exasol start` picks it up; the SQL port is read fresh because it is assigned per deployment via `exasol config set --ports db:<port>`, so a hardcoded value registers against another deployment's database. Remove the phrase `both reassigned ports`.
- [x] 4.3 In `scripts/install.sh`, extend the doc comment above `resolve_local_connection` (lines 272-276) with a leading sentence stating the contract in the same shape as `resolve_cloud_connection`'s: it finalizes the connection globals `HOST` and `PORT` for a local Exasol Personal deployment from its `deployment.json`, with an explicit `--port` (`CLI_PORT`) winning over `.connection.dbPort` and `PERSONAL_DB_PORT_DEFAULT` used when the field is absent. Keep the existing design-intent and returns-never-exits sentences.
- [x] 4.4 In `scripts/install.sh` line 245, replace the literal `'8563'` with `"$PERSONAL_DB_PORT_DEFAULT"` in the `deployment_field "$dir" '.connection.dbPort' …` call inside `resolve_cloud_connection`. Change nothing else in that function.
- [x] 4.5 In `docs/installation.md` line 125, change `` `scp` over that port `` to `` `scp` over the SSH port ``.

## Phase 5: Unify connection resolution (Group D)
- [x] 5.1 Retarget the nine existing resolver tests in `scripts/tests/install-personal-test.sh`: five cloud and three local tests call `resolve_deployment_connection` (cloud passes `''` as the default host, local passes `"$PERSONAL_DB_HOST_DEFAULT"`) and `cloud_requires_operator_bfs_password` calls `require_cloud_bfs_password`; add a `secrets.json` fixture to all three local tests; add `PORT="unresolved"`/`USER="unresolved"` sentinels to `resolves_cloud_defaults_when_connection_fields_absent`; invert `resolves_local_connection_from_descriptor`'s `HOST` assertions to `descriptor.example`, add its `PASSWORD` assertion, and set `HOST=""` before the missing-descriptor resolve (plan.md task 5) [expert]

## Phase 5: Unify connection resolution (Group E)
- [x] 5.2 Add `cli_host_overrides_local_descriptor`, `resolves_local_host_default_when_absent`, and `resolves_local_user_from_descriptor` to `scripts/tests/install-personal-test.sh` and register them in the `run` list (plan.md task 6)

## Phase 5: Unify connection resolution (Group F)
- [x] 5.3 Replace `resolve_cloud_connection` and `resolve_local_connection` in `scripts/install.sh` with `resolve_deployment_connection <dir> <default_host>`, add `require_cloud_bfs_password`, and rename `PERSONAL_DB_HOST` to `PERSONAL_DB_HOST_DEFAULT` together with its `usage()` reference (plan.md task 7) [expert]

## Phase 5: Unify connection resolution (Group G)
- [x] 5.4 Rewire both `main` call sites, delete the inline local `secrets.json` password block and its `die`, and add the missing-`.connection.dbPort` warning to the local branch (plan.md task 8)

## Phase 5: Unify connection resolution (Group H)
- [x] 5.5 Print `${HOST}:${PORT}` in both registration banners (plan.md task 9)
- [x] 5.6 Correct the host prose in `docs/installation.md` § Exasol Personal install (plan.md task 11)

## Phase 5: Unify connection resolution (Group I)
- [x] 5.7 Correct the host prose in `scripts/install.sh` — the `-D, --deployment` local `usage()` block and the transport-selection comment (plan.md task 10)

## Phase 6: Verification
- [x] 6.1 Run `bash scripts/tests/install-personal-test.sh`
- [~] 6.2 Run `shellcheck scripts/install.sh scripts/tests/install-personal-test.sh` — not installed in this sandbox, no passwordless sudo; unresolved, needs CI or a manual run
- [x] 6.3 Run `cargo build --release`
- [x] 6.4 Run `cargo test`
- [x] 6.5 Run `cargo clippy --all-targets --all-features -- -D warnings`
- [x] 6.6 Run `cargo fmt --check`

## Phase 5b: Review Fixes (Expert)
- [x] 5b.1 In `scripts/tests/install-personal-test.sh`, add `cloud_requires_host_when_descriptor_omits_it` immediately above `cloud_requires_db_password` and register it directly above `run cloud_requires_db_password`: a host-less cloud descriptor plus `secrets.json`, resolved via `resolve_deployment_connection "$dir" ''`, MUST return 1 and leave `HOST` empty, asserting the cloud call site's empty `default_host` [expert]
- [x] 5b.2 In `scripts/tests/install-personal-test.sh`, function `resolves_cloud_connection_from_descriptor`: add `PORT="unresolved"` and `USER="unresolved"` sentinels on the two lines immediately after `reset_connection_globals` so its PORT/USER assertions can fail [expert]
- [x] 5b.3 In `scripts/install.sh`, function `resolve_deployment_connection`: add an unreadable-descriptor guard clause as the first statement after the `local` declarations and update the doc comment to state it returns 1 on an unreadable `deployment.json` and that the guard is what makes the three descriptor defaults meaningful; add the test `unreadable_descriptor_fails_even_with_cli_overrides` to `scripts/tests/install-personal-test.sh` (written first, confirmed failing against the unguarded resolver), registered directly below `run resolves_local_connection_from_descriptor` [expert]

## Phase 5b: Review Fixes (Standard)
- [x] 5b.4 In `scripts/tests/install-personal-test.sh`, function `cloud_requires_operator_bfs_password`: append after the existing checks a `BFS_PASSWORD="bfspw"` assignment, a `require_cloud_bfs_password` call, and a `check "a supplied --bfs-password lets the cloud install proceed" "0" "$?"` assertion; rename the two existing check labels from `"an empty --bfs-password fails cloud resolution"` to `"an empty --bfs-password fails the cloud BucketFS-password requirement"` and from `"the error names --bfs-password as the missing Personal requirement"` to `"the error names --bfs-password as the missing BucketFS credential"`
- [x] 5b.5 In `scripts/tests/install-personal-test.sh`, delete the dead `BFS_PASSWORD="bfspw"` assignment from each of `resolves_cloud_connection_from_descriptor`, `cli_flags_override_cloud_descriptor`, `cloud_requires_db_password`, `resolves_cloud_defaults_when_connection_fields_absent`, and `cloud_leaves_scope_untouched` — `resolve_deployment_connection` no longer reads that variable
- [x] 5b.6 In `scripts/tests/install-personal-test.sh`, function `resolves_local_connection_from_descriptor`: delete the tautological second `check "HOST resolves from connection.host" "descriptor.example" "$HOST"` assertion that follows the second resolve, keeping the first HOST assertion, the dbPort-freshness assertion, and the `HOST=""` line with its comment
