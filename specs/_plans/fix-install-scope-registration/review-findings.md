# Code Review Findings: fix-install-scope-registration

## Summary
- Files reviewed: 4
- Total findings: 7 (standard: 6, expert: 1)

Reviewed: `docs/installation.md`, `scripts/install.sh`, `scripts/tests/install-personal-test.sh`, `specs/mission.md`.

Evidence gathered:
- `bash scripts/tests/install-personal-test.sh` → `All assertions passed.` (exit 0); all four registration tests report `ok`.
- `shellcheck` is not installed in this environment, so the plan's Checklist shellcheck row could not be executed. Not raised as a finding; noted for the orchestrator.
- `specs/mission.md` is correct as changed. No findings against it.

## Standard fixes

### scripts/install.sh

#### [OUTDATED_COMMENT] The file header still offers `ALTER SESSION` as a registration scope
- Location: lines 8 and 22
- Issue: line 8 reads `BucketFS HTTP API and register with ALTER SESSION|SYSTEM.` After this change no transport can register at `SESSION` scope, so the first paragraph a maintainer reads describes a removed capability. Line 22 reads `ALTER SYSTEM and preserves every pre-existing SCRIPT_LANGUAGES entry` inside the `"local"` backend bullet, presenting as a local-backend property what is now the behavior of every transport. The plan's own round-2 review recorded both sites as advisory (`specs/_plans/fix-install-scope-registration/review/round-2.md:33`) and neither was edited.
- Fix: In `scripts/install.sh`, change line 8 to end `upload the tarball over the BucketFS HTTP API.` and delete the trailing `register with ALTER SESSION|SYSTEM` clause. Delete the sentence `Registration uses ALTER SYSTEM and preserves every pre-existing SCRIPT_LANGUAGES entry.` from the `"local"` bullet (lines 21-22). Add one line to the two-transport preamble, directly above the `* Default` bullet (after line 5), stating that both transports register through `register_script_languages`, which merges the `RUST` entry into the current value and writes it with `ALTER SYSTEM`.

#### [OUTDATED_COMMENT] Three comments still name a per-arm ALTER scope that no longer exists
- Location: lines 367, 601, 623
- Issue: line 367 lists `the local ALTER scope` among the things that are "not a connection field" and therefore "stay at the call site" — there is no local ALTER scope left to stay anywhere, so the enumeration names a deleted concept. Line 601 annotates the local branch with `(SQL endpoint resolved from the descriptor with a 127.0.0.1 host default, ALTER SYSTEM)` and lines 623-624 read `Personal-local always talks to the VM's local SQL port; ALTER SYSTEM so the registration survives a restart.` Both present `ALTER SYSTEM` as what distinguishes the local arm, which is exactly the distinction this change removed. The plan's round-1 review recorded line 367 as advisory (`review/round-1.md:45`).
- Fix: In `scripts/install.sh`, at line 367 remove `the local ALTER scope, ` from the parenthesised list so it reads `(the missing-dbPort warning, the cloud BucketFS password)`. At line 601 remove `, ALTER SYSTEM` from the parenthesis. At lines 623-624 replace the comment with `Personal-local always talks to the VM's local SQL port; only placement differs by mechanism.`

#### [SHRINKABLE] The read-failure guard restates the message its callee already printed
- Location: lines 519-520
- Issue: `register_script_languages`'s guard echoes `error: cannot read the current SCRIPT_LANGUAGES value; refusing to register`, which `current_script_languages` (line 506) has already printed with more context. Both call sites then add a third line through `|| die "cannot register the RUST language"` (lines 701 and 723). Measured on a stubbed `exapump` that exits 1 on the read, `register_script_languages` alone emits:
  ```
  exapump: could not connect to host
  error: cannot read the current SCRIPT_LANGUAGES value from db.example:8563
  error: cannot read the current SCRIPT_LANGUAGES value; refusing to register
  ```
  The second `error:` line adds no information the first does not carry; `parse_script_languages` (line 463) already supplies the "refusing to register" consequence on the other read-failure branch. The plan specified only that the function "returns `1` when that read fails".
- Fix: In `scripts/install.sh`, replace lines 519-520 with `existing="$(current_script_languages "$dsn")" || return 1` and delete the `echo` from that guard.

#### [UNTESTED_ERROR_PATH] No test covers `register_script_languages` when the ALTER statement fails
- Location: line 524
- Issue: `register_script_languages` has two failure modes and only the read failure is tested. The `exapump sql "ALTER SYSTEM …"` call is the function's last command, so its exit status becomes the function's return status, which both call sites convert to a `die`. That propagation is load-bearing and untested: the function is invoked as `register_script_languages … || die …`, and a `||` condition context disables `errexit` inside the function body, so nothing but the return status stops the script from printing `==> Done. The RUST script language is now available.` after a rejected `ALTER`.
- Fix: In `scripts/tests/install-personal-test.sh`, add `fails_when_the_alter_statement_is_rejected` beside `refuses_to_alter_when_the_current_value_cannot_be_read` (line 349). Give its `exapump` stub the same `*SELECT*` branch as `registers_by_merging_into_alter_system` (printing `CURRENT_SCRIPT_LANGUAGES` then `PYTHON3=builtin_python3 JAVA=builtin_java`) plus an `*ALTER*` branch that writes an error to stderr and exits `1`. Assert `register_script_languages` returns `1`, and assert the call log records the `ALTER SYSTEM` line so the test proves the statement was attempted rather than skipped. Register it in the `run` list after line 805.

### scripts/tests/install-personal-test.sh

#### [VAGUE_TEST_NAME] Two adjacent tests carry near-identical names for different subjects
- Location: lines 274 and 323
- Issue: `refuses_to_register_when_the_current_value_cannot_be_read` (line 274) exercises `current_script_languages` and `parse_script_languages` — it asserts that the *read* fails and never calls a registration function. This change added `refuses_to_alter_when_the_current_value_cannot_be_read` (line 323), which does exercise `register_script_languages`. The two names now differ by one verb while their subjects and assertions differ entirely, and both appear two lines apart in the `run` list (lines 803 and 805), so neither name states which behavior it pins.
- Fix: In `scripts/tests/install-personal-test.sh`, rename the function at line 274 to `refuses_to_read_a_current_value_the_query_cannot_supply` and update its `run` entry at line 803 to match. Change no assertions inside it.

### docs/installation.md

#### [MISSING_DOC_COMMENT] The newly required database privileges are documented nowhere
- Location: lines 55-57
- Issue: the new paragraph states that the install reads the current value and persists the result with `ALTER SYSTEM`, but not what that now requires of the DB user given to `--user`/`--password`. Two privileges became mandatory on every transport: `SELECT` on `EXA_PARAMETERS` (the read is a hard failure, per `current_script_languages` at `scripts/install.sh:506`) and the right to run `ALTER SYSTEM`. The manual-install section flags the latter explicitly (`docs/installation.md:321`, `-- persists across sessions (requires admin)`), so the automated path is the only one that leaves it unsaid. Before this change a non-admin user could complete the default install at `SESSION` scope; now it fails. `usage()` in `scripts/install.sh` (lines 82-84) has the same gap: it announces the `ALTER SYSTEM` behavior without its prerequisite.
- Fix: In `docs/installation.md`, extend the paragraph at lines 55-57 with one sentence stating that the user passed to `--user`/`--password` needs `SELECT` on `EXA_PARAMETERS` and the privilege to run `ALTER SYSTEM`, and that the install aborts without registering if the read fails. In `scripts/install.sh`, add one line to the `usage()` paragraph at lines 82-84 stating the same two privileges.

## Expert fixes

### scripts/install.sh

#### [AMBIENT_STATE_READ] The registration banner reads `HOST`/`PORT` globals instead of its own `dsn` argument
- Location: line 523
- Issue: `register_script_languages` receives the endpoint as its `dsn` argument but prints `==> Registering RUST at ${HOST}:${PORT} …` from two globals that are not part of its interface, so the banner can name an endpoint other than the one it writes to. This is not hypothetical: in the passing test run, `registers_by_merging_into_alter_system` (line 312 of the test file) passes `exasol://sys:x@127.0.0.1:8563` and the banner prints
  ```
  ==> Registering RUST at 127.0.0.1:52164 (ALTER SYSTEM SET SCRIPT_LANGUAGES) …
  ```
  because `PORT=52164` survives from an earlier test's deployment fixture; the new tests never call `reset_connection_globals`. The function is now the single owner of registration for both transports, so the hidden dependency on two caller-set globals is the one thing about it a call site cannot see from its signature. The assertions do not catch it because none of them looks at the banner.
- Fix: In `scripts/install.sh`, derive the endpoint inside `register_script_languages` from its own `dsn` argument instead of `HOST`/`PORT`: strip everything through the first `@` and everything from the first `?`, and print that value in the banner. The `dsn` carries the DB password before the `@` (`exasol://${USER}:${PASSWORD}@${HOST}:${PORT}?…`, lines 688 and 721), so the extraction MUST drop the credential prefix — never echo the `dsn` itself or any prefix of it, in this banner or in an error. Write the test first: in `scripts/tests/install-personal-test.sh`, extend `registers_by_merging_into_alter_system` to capture the function's stdout, assert it contains `Registering RUST at 127.0.0.1:8563`, and assert it contains neither `sys:x` nor the literal password from the fixture dsn. Set `PORT` to a different value than the dsn's port immediately before the call, so the test fails against the current global-reading banner.
