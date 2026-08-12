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
