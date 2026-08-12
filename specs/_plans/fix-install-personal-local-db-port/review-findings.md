# Code Review Findings: fix-install-personal-local-db-port

## Summary
- Files reviewed: 3
- Total findings: 5 (standard: 5, expert: 0)

Verified before reviewing: `bash scripts/tests/install-personal-test.sh` → `All assertions passed.` (47 `ok`, exit 0); `bash scripts/install.sh --help` → exit 0 with the renamed constant interpolated (no `set -u` abort); `grep -rn 'PERSONAL_DB_PORT' --include='*.sh' --include='*.md'` outside `specs/_plans` → only `scripts/install.sh:43,96,279`, all `PERSONAL_DB_PORT_DEFAULT`, so the rename left no dangling reference. `shellcheck` is not installed on this host, so checklist step 3.2 remains unverified. The core defect is fixed correctly: the local branch now resolves `PORT` through `deployment_field '.connection.dbPort'` with `CLI_PORT` winning, matching the cloud precedence.

## Standard fixes

### scripts/tests/install-personal-test.sh

#### [MISSING_BOUNDARY_TEST] The newly documented "`--host` is ignored for local" contract is unfalsifiable
- Location: line 118 (assertion), lines 109 and 122 (fixtures)
- Issue: `usage()` now states a new contract — `SQL host is fixed at 127.0.0.1 (so --host is ignored)` (`scripts/install.sh:93`) — and `resolve_local_connection` implements it by assigning `HOST="$PERSONAL_DB_HOST"` unconditionally (`scripts/install.sh:281`). No assertion can distinguish that from the two wrong implementations, because `reset_connection_globals` leaves `HOST=""` (`:191`) and the fixture descriptor's `.connection.host` is `127.0.0.1` — byte-identical to `PERSONAL_DB_HOST`. Confirmed by mutation: replacing line 281 with `[[ -z "$HOST" ]] && HOST="$PERSONAL_DB_HOST"` (which makes `--host` silently take effect, contradicting `usage()`) leaves all 47 assertions green (exit 0); replacing it with `HOST="$(deployment_field "$dir" '.connection.host' "$PERSONAL_DB_HOST")"` also leaves all 47 green. This is the same vacuous shape the plan guarded `PORT` against with the `PORT="unresolved"` sentinel, applied to `HOST` instead — the check named "HOST is fixed at the local forwarder, not read from the descriptor" currently proves neither clause.
- Fix: In `scripts/tests/install-personal-test.sh`, function `resolves_local_connection_from_descriptor`: change `.connection.host` in both descriptor `printf` payloads (lines 109 and 122) from `"127.0.0.1"` to `"descriptor.example"`; after `reset_connection_globals` (line 112) add `HOST="override.example"` alongside the existing `PORT="unresolved"` sentinel, and re-set `HOST="override.example"` next to the second `PORT="unresolved"` (line 121) before the reassigned-`dbPort` resolve; keep both `HOST` assertions expecting `127.0.0.1` and rename the line-118 check text to `"HOST is fixed at the local forwarder, overriding both --host and the descriptor"`; add the same `HOST` assertion after the second resolve. Do not change the descriptor host in `cli_port_overrides_local_descriptor` or `resolves_local_defaults_when_db_port_absent`. Then re-run `bash scripts/tests/install-personal-test.sh` and confirm it still reports `All assertions passed.`, and confirm the two mutations above now fail.

### scripts/install.sh

#### [OUTDATED_COMMENT] File-header comment claims `exasol stop && exasol start` reassigns the SQL port
- Location: lines 20-21
- Issue: the rewritten local bullet ends `re-running after \`exasol stop && exasol start\` picks up both reassigned ports`. Only the SSH port is reassigned by a stop/start cycle. The SQL port is a per-deployment assignment made with `exasol config set --ports db:<port>` and is stable across restarts — exactly as this plan's own spec delta records (`... after an \`exasol stop\`/\`start\` cycle that reassigns the SSH port and after an \`exasol config set --ports db:<port>\` that reassigns the SQL port`) and as `docs/installation.md:128-130` still correctly says (`it picks up the reassigned SSH port`). The comment therefore gives a maintainer the wrong reason for reading `connection.dbPort` fresh, and contradicts the docs page changed in the same commit.
- Fix: In `scripts/install.sh`, rewrite the trailing clause of the local bullet (lines 17-21) so the two ports have their own reasons: the SSH port is reassigned on every `exasol start`, so re-running after `exasol stop && exasol start` picks it up; the SQL port is read fresh because it is assigned per deployment via `exasol config set --ports db:<port>`, so a hardcoded value registers against another deployment's database. Remove the phrase `both reassigned ports`.

#### [MISSING_DOC_COMMENT] `resolve_local_connection`'s doc comment never names its outputs
- Location: lines 272-276
- Issue: the function's entire result is delivered by assigning the globals `HOST` and `PORT`, yet the comment states only why the port must be resolved and that the function returns rather than exits. A caller cannot learn from it what the function promises. Its sibling `resolve_cloud_connection` opens with exactly that contract — `Finalize the normal-path connection globals HOST/PORT/USER/PASSWORD for …` (line 226) — so the new function is the only one of the pair whose comment omits it, and the `--port`-wins precedence it owns is undocumented too.
- Fix: In `scripts/install.sh`, extend the doc comment above `resolve_local_connection` (lines 272-276) with a leading sentence stating the contract in the same shape as `resolve_cloud_connection`'s: it finalizes the connection globals `HOST` and `PORT` for a local Exasol Personal deployment from its `deployment.json`, with an explicit `--port` (`CLI_PORT`) winning over `.connection.dbPort` and `PERSONAL_DB_PORT_DEFAULT` used when the field is absent. Keep the existing design-intent and returns-never-exits sentences.

#### [MAGIC_NUMBER] Cloud's `.connection.dbPort` default is still the bare literal `'8563'`
- Location: line 245
- Issue: `resolve_local_connection` passes `"$PERSONAL_DB_PORT_DEFAULT"` for the `.connection.dbPort` fallback (line 279) while `resolve_cloud_connection` still passes `'8563'` for the same field's fallback (line 245). One decision — the descriptor SQL-port default — now has two spellings in one file, one of them an unnamed literal, so changing it means editing both sites. The rename performed by this change is what made the constant applicable to both backends: it sits under `Personal-transport constants` (line 43) and its name carries no backend. Note this contradicts the plan's Non-Goals, which scoped the cloud branch out and chose to keep the literal; the substitution below is behaviour-preserving and is the smaller of the two costs.
- Fix: In `scripts/install.sh` line 245, replace the literal `'8563'` with `"$PERSONAL_DB_PORT_DEFAULT"` in the `deployment_field "$dir" '.connection.dbPort' …` call inside `resolve_cloud_connection`. Change nothing else in that function, then re-run `bash scripts/tests/install-personal-test.sh` and confirm `resolves_cloud_defaults_when_connection_fields_absent` still passes.

### docs/installation.md

#### [OUTDATED_COMMENT] "scp over that port" became ambiguous once the row above listed two ports
- Location: line 125
- Issue: the "Copy and extract" row reads `` `scp` over that port ``. Before this change the preceding row named exactly one port (`connection.sshPort`), so the back-reference was unambiguous. That row now lists `connection.sshPort`, `connection.dbPort`, and the node key, so "that port" can be read as the SQL port — which `scp` never uses.
- Fix: In `docs/installation.md` line 125, change `` `scp` over that port `` to `` `scp` over the SSH port ``.

## Expert fixes
[none]
