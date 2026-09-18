# Tasks: fix-install-scope-registration

## PR Lifecycle
- [x] resolved
- [x] implemented
- [x] version-bumped — no bump: scripts/install.sh is deployment tooling, not language-container production code (see decision-log.md Design Decision [2]); `[workspace.package].version` stays `0.28.1`
- [ ] tested-green
- [ ] recorded
- [ ] pr-ready

## Phase 2: Implementation (Group A: registration consolidation)
- [x] 2.1 Replace the scope assertions in `scripts/tests/install-personal-test.sh` with assertions on the shared registration function: delete `cloud_leaves_scope_untouched` and its `run` line, delete `SCOPE=SESSION` from `reset_connection_globals`; add `registers_by_merging_into_alter_system`, `refuses_to_alter_when_the_current_value_cannot_be_read`, and `both_transports_call_the_shared_registration`; register all three in the `run` list
- [x] 2.2 Add `register_script_languages <dsn> <entry>` to `scripts/install.sh`, call it from both arms of task 3, rename the `# ── Personal: SCRIPT_LANGUAGES value handling ──` section header
- [x] 2.3 Remove the `--scope` surface from `scripts/install.sh` (default, arg-loop case, override, `SCOPE_UPPER` validation block, `usage()` lines, SaaS example); run `shellcheck scripts/install.sh`
- [x] 2.4 Update `docs/installation.md`: replace the cloud paragraph, add a sentence to the automated-install section about read-merge-persist; leave the manual-install section unchanged
- [x] 2.5 Sync `specs/mission.md`: `ALTER SESSION` to `ALTER SYSTEM` at the two named lines, restate the glossary row as the general Exasol mechanism

## Phase 3: Verification
- [x] 3.1 Run `bash scripts/tests/install-personal-test.sh`
- [x] 3.2 Run `shellcheck scripts/install.sh scripts/tests/install-personal-test.sh`
- [x] 3.3 Run `cargo build --release`
- [x] 3.4 Run `cargo test`
- [x] 3.5 Run `cargo clippy --all-targets --all-features -- -D warnings`
- [x] 3.6 Run `cargo fmt --check`
- [x] 3.7 Confirm `git diff --stat Cargo.toml Cargo.lock` is empty (no version bump)
- [x] 3.8 Manual: default-path install against a live docker-db (entry preservation, persistence, idempotency, `--scope` removal)

## Phase 4: Review Fixes
- [x] 4.1 Rewrite the `scripts/install.sh` file header: end line 8 at `upload the tarball over the BucketFS HTTP API.`, delete the `Registration uses ALTER SYSTEM and preserves every pre-existing SCRIPT_LANGUAGES entry.` sentence from the `"local"` bullet, and add one line above the `* Default` bullet stating that both transports register through `register_script_languages`, which merges the `RUST` entry into the current value and writes it with `ALTER SYSTEM`
- [x] 4.2 Drop the per-arm ALTER scope from the three remaining `scripts/install.sh` comments: remove `the local ALTER scope, ` at line 367, remove `, ALTER SYSTEM` at line 601, and replace the lines 623-624 comment with `Personal-local always talks to the VM's local SQL port; only placement differs by mechanism.`
- [x] 4.3 Collapse the `register_script_languages` read-failure guard in `scripts/install.sh` to `existing="$(current_script_languages "$dsn")" || return 1`, deleting the echo its callee already printed
- [x] 4.4 Add `fails_when_the_alter_statement_is_rejected` to `scripts/tests/install-personal-test.sh` with a stub whose `*SELECT*` branch returns the current value and whose `*ALTER*` branch fails, asserting `register_script_languages` returns `1` and that the call log records the attempted `ALTER SYSTEM`; register it in the `run` list
- [x] 4.5 Rename `refuses_to_register_when_the_current_value_cannot_be_read` to `refuses_to_read_a_current_value_the_query_cannot_supply` in `scripts/tests/install-personal-test.sh` and update its `run` entry, changing no assertions
- [x] 4.6 Document the two newly mandatory privileges: extend the automated-install paragraph in `docs/installation.md` with the `SELECT` on `EXA_PARAMETERS` plus `ALTER SYSTEM` requirement and the abort-without-registering consequence, and add the same line to `usage()` in `scripts/install.sh`
- [x] 4.7 Derive the registration banner's endpoint inside `register_script_languages` from its own `dsn` argument instead of the `HOST`/`PORT` globals, dropping the credential prefix so no part of the dsn is ever echoed, with the banner assertions written first in `registers_by_merging_into_alter_system` [expert]
