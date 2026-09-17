# Code Review Findings: fix-personal-slc-install

## Summary
- Files reviewed: 10 (Cargo.lock, Cargo.toml, Dockerfile, README.md, dist/slc-sandbox-skeleton.txt, dist/tests/slc_tarball_test.sh, docs/installation.md, docs/writing-a-udf.md, scripts/install.sh, scripts/tests/install-personal-test.sh)
- Total findings: 6 (standard: 5, expert: 1)

Evidence gathered: `bash scripts/tests/install-personal-test.sh` → `All assertions passed`; `shellcheck -x` on all three changed shell scripts reports no new diagnostic (only the pre-existing SC1091/SC2034/SC2053/SC2086/SC2020 set); `Cargo.lock` carries the `0.28.0` → `0.28.1` bumps only.

## Pre-existing-bug triage (requested by the orchestrator)

The implementer flagged `EXISTING="$(current_script_languages "$DSN")"` (`scripts/install.sh` line 744) as a pre-existing hazard. **Verified reproducible, and raised below as an in-scope Expert finding.**

The stated mechanism is slightly off: a bare assignment from a command substitution *does* trip `set -e` (`X="$(false)"` exits 1). The actual mechanism is that `errexit` is suppressed *inside* a command substitution that feeds an assignment, so the failure never reaches the assignment at all. Reproduced against the real functions on bash 3.2 (the macOS default this path runs on, per the script's own comment at line 690) and on bash 5:

```
$ PATH=<stub-exapump-that-exits-1>:$PATH bash -c 'source scripts/lib/script_languages.sh; source scripts/install.sh;
    f() { ENTRY="$(script_languages_entry bfsdefault default rustslc)"
          EXISTING="$(current_script_languages "exasol://sys:x@127.0.0.1:8563")"
          echo "WOULD RUN: ALTER SYSTEM SET SCRIPT_LANGUAGES=$(script_languages_with_rust_entry "$EXISTING" "$ENTRY")"; }; f'
exapump: could not connect to host
WOULD RUN: ALTER SYSTEM SET SCRIPT_LANGUAGES='RUST=localzmq+protobuf:///bfsdefault/default/rustslc?lang=rust#buckets/bfsdefault/default/rustslc/exaudf/exaudfclient'
exit=0
```

In scope, for three reasons: `scripts/install.sh` is in the changed-files list; this plan is what makes the local branch reachable for the whole population of deployments that publish no SSH inputs; and the plan's own Verification table claims `personal-install: Registration is system-scoped and preserves existing entries`, which this path silently violates. The plan also already treats `--bfs-service`/`--bucket`/`--slc-name` hardening as in-scope collateral on the same code path.

## Standard fixes

### Dockerfile

#### [SWALLOWED_ERROR] The sandbox-skeleton loop ignores a failing mkdir and fails the build on a trailing blank line
- Location: lines 129-131
- Issue: `RUN while IFS= read -r dir; do [ -n "$dir" ] && mkdir -p "/slc/$dir"; done < /slc-meta/sandbox-skeleton` has two defects. (1) A `mkdir -p` that fails mid-file does not fail the step: the `while` loop's status is only the last iteration's, so the build continues with an incomplete skeleton. The sibling staging loop at line 100 (`for lib in $(cat /slc-meta/library-surface); do cp -L ... || exit 1; done`) already guards exactly this. (2) The `[ -n "$dir" ] &&` guard exists to tolerate blank lines, but it makes the loop's own exit status 1 whenever the file's last line is blank, failing the build instead. Verified: `sh -c 'while IFS= read -r d; do [ -n "$d" ] && mkdir -p "/tmp/skt/$d"; done < f'` returns `1` for a file ending in a blank line and `0` otherwise. The committed `dist/slc-sandbox-skeleton.txt` happens to have no trailing blank line today, so the image builds; adding one breaks it.
- Fix: In Dockerfile, replace the `RUN` body at lines 129-131 with a loop that skips a blank line without leaving it as the loop's status and that fails on a failing mkdir, e.g. `RUN while IFS= read -r dir; do if [ -n "$dir" ]; then mkdir -p "/slc/$dir" || exit 1; fi; done < /slc-meta/sandbox-skeleton`. Rebuild with `docker build --target artifact --output type=local,dest=/tmp/lc-out .` and re-run `bash dist/tests/slc_tarball_test.sh /tmp/lc-out/lc-rs.tar.gz` to confirm the skeleton still ships.

### dist/tests/slc_tarball_test.sh

#### [ASSERTION_FREE_TEST] slc_tarball_ships_sandbox_skeleton passes vacuously on an empty skeleton file
- Location: lines 632-653
- Issue: the test loops over `$SANDBOX_SKELETON_FILE` and calls `pass` after the loop. If the file exists but is empty, whitespace-only, or truncated to zero names, the loop body never runs and the test reports `pass` with zero assertions executed — the same truncation would also make the Dockerfile stage an empty skeleton, so the two failures cancel and CI stays green. The sibling `slc_tarball_library_surface_present` (lines 351-362) guards precisely this with `if [[ "${#surface[@]}" -eq 0 ]]; then fail "... names no library"; return; fi`.
- Fix: In dist/tests/slc_tarball_test.sh, rewrite `slc_tarball_ships_sandbox_skeleton` to collect the parsed names into a local array first (mirroring `slc_tarball_library_surface_present`), `fail` with `"slc_tarball_ships_sandbox_skeleton: $SANDBOX_SKELETON_FILE names no directory"` and `return` when the array is empty, and only then loop over the array for the directory and non-directory-entry checks.

### scripts/install.sh

#### [OUTDATED_COMMENT] The failure-status constants claim a per-status message the caller does not produce
- Location: lines 46-49
- Issue: the comment reads `deployment_bucketfs_dir failure statuses. Each is a different operator error, so the caller reports a different message for each rather than one catch-all.` The only caller, `personal_local_mechanism`, branches on `BUCKETFS_MAPPING_ABSENT` alone (line 326) and sends statuses 3 and 1 down the identical pass-through branch (line 329). `grep -rn BUCKETFS_PAIR_UNSERVED scripts/` shows it is defined (line 49), returned (line 263) and named in a comment (line 238), and never compared anywhere. The function's own doc comment at lines 308-310 states the real behaviour and contradicts this one.
- Fix: In scripts/install.sh, rewrite the comment at lines 46-47 to match the code: state that `BUCKETFS_MAPPING_ABSENT` is the only status the caller branches on (it means neither local mechanism applies, so the caller writes its own combined message), and that `BUCKETFS_PAIR_UNSERVED` and status 1 are distinguished so `deployment_bucketfs_dir` can name the failure itself while the caller passes its message through unchanged.

#### [MISSING_DESIGN_INTENT] personal_local_mechanism's doc comment omits the errexit contract every sibling documents
- Location: lines 298-333 (doc comment), line 319 (`err="$(deployment_bucketfs_dir "$dir" "$service" "$bucket" 2>&1 >/dev/null)"`)
- Issue: line 319 captures stderr and lets the assignment carry a non-zero status. Under `set -euo pipefail` that is safe only because the single call site (line 667) is `LOCAL_MECHANISM="$(personal_local_mechanism …)" || exit 1`, whose command-substitution context suppresses `errexit` inside. Called from any ordinary statement position the function would abort the script at line 319 with nothing printed, because the operator-facing message is in `$err` and has not been re-emitted yet. Every other function in this file that depends on its caller's context says so explicitly: `resolve_deployment_connection` (lines 404-407), `require_cloud_bfs_password` (lines 459-460), `extract_slc_into_shared_bucketfs` (lines 566-568).
- Fix: In scripts/install.sh, append a paragraph to `personal_local_mechanism`'s doc comment, in the wording the three sibling functions use, stating that it returns 1 and never exits, that line 319 deliberately captures `deployment_bucketfs_dir`'s stderr so the status can be inspected, and that the function must therefore be invoked from a command-substitution or condition context (as `main` does at line 667) so `errexit` does not abort before the message is re-emitted.

### scripts/tests/install-personal-test.sh

#### [ASSERTION_FREE_TEST] The "both mechanisms assemble the same entry" check compares a value with itself
- Location: lines 722-723, 733-735
- Issue: `ssh_entry` and `shared_entry` are assigned from two textually identical calls, `script_languages_entry "$BFS_SERVICE" "$BUCKET" "$SLC_NAME"`, with no mechanism-derived input on either side. `check "both mechanisms assemble the same SCRIPT_LANGUAGES entry" "$ssh_entry" "$shared_entry"` therefore cannot fail under any implementation change, and the comment above it at lines 733-734 (`a future change that fed a mechanism-derived path into it would break these`) states the opposite of what the code does. The real contract is already carried by the two `*/"$bucket_path"` destination checks (lines 727-731) and by `check "that entry is the Personal bucket-root entry" "$PERSONAL_ENTRY" "$shared_entry"`.
- Fix: In scripts/tests/install-personal-test.sh, in `local_mechanisms_share_the_registration_inputs`, delete the `ssh_entry`/`shared_entry` duplicate assignments at lines 722-723, delete the tautological `check` at line 735 and the false comment at lines 733-734, and rewrite the two remaining entry assertions to call `script_languages_entry "$BFS_SERVICE" "$BUCKET" "$SLC_NAME"` once into a single `entry` variable that both the `$PERSONAL_ENTRY` check and the fragment check use. Remove `ssh_entry` and `shared_entry` from the function's `local` declaration. Re-run `bash scripts/tests/install-personal-test.sh` and confirm `All assertions passed`.

### docs/installation.md

#### [OUTDATED_COMMENT] The manual .so copy snippet hard-codes the shared bucket path the install refuses to hard-code
- Location: line 120
- Issue: the snippet sets `bucket=~/.exasol/personal/deployments/<name>/local/runtime/vm-shared/exa/bucketfs/bfsdefault/default`, restating the exact layout `deployment_bucketfs_dir` was written not to restate: `scripts/install.sh` lines 230-233 record that `Rebuilding the layout from a hard-coded path here would be a second copy of that declaration, and a service/bucket pair the deployment does not serve would become a directory the engine never reconciles instead of an error.` An operator whose `bucketfs.conf` names a different VM-side path for `bfsdefault`/`default` follows this snippet into a directory the engine never reconciles, and `cp` reports success.
- Fix: In docs/installation.md, in the "Shared host directory" block around line 120, add one sentence before the fenced snippet stating that the path shown is the usual one and that the authoritative host directory for a given `<service>`/`<bucket>` is the one the deployment's own `local/runtime/vm-shared/exa/bucketfs.conf` names, which is where `scripts/install.sh` reads it from. Keep the block within its current length.

## Expert fixes

### scripts/install.sh

#### [SWALLOWED_ERROR] A failed SCRIPT_LANGUAGES read registers RUST as the only script language
- Location: line 744 (`EXISTING="$(current_script_languages "$DSN")"`), with the root cause in `current_script_languages` (lines 526-533) and `parse_script_languages` (lines 485-505)
- Issue: `current_script_languages` runs `output="$(exapump sql …)"` and then `parse_script_languages "$output"`. Because the whole function is itself invoked from a command substitution at line 744, `errexit` is suppressed inside it, so a failing `exapump` leaves `output` empty and execution falls through to `parse_script_languages ""`, which finds no data line, produces an empty `value`, runs its token loop zero times and returns 0. `EXISTING` is then the empty string, `script_languages_with_rust_entry "" "$ENTRY"` yields the RUST entry alone (asserted today by `preserves_existing_script_languages`: `an empty parameter yields the RUST entry alone`), and line 748 executes `ALTER SYSTEM SET SCRIPT_LANGUAGES='RUST=…'`, dropping every pre-existing language system-wide. The same fall-through occurs for a header-only or otherwise data-free result, which `parse_script_languages`' own comment at lines 482-484 claims it prevents (`An unrecognized shape is an error rather than an empty value: silently treating it as empty would drop every language the database already has`) — the guard only rejects a *malformed* token, never an absent one. Reproduced against the real functions on bash 3.2 and bash 5; see the triage section above for the transcript.
- Fix: In scripts/install.sh, close the hole test-first. (1) Add a failing test to scripts/tests/install-personal-test.sh named `refuses_to_register_when_the_current_value_cannot_be_read`: put a stub `exapump` that prints to stderr and exits 1 on a temporary `PATH` entry, call `current_script_languages "exasol://sys:x@127.0.0.1:8563"`, and assert exit status 1; add two assertions driving `parse_script_languages` directly with an empty string and with header-only output (`CURRENT_SCRIPT_LANGUAGES` alone), both asserting exit status 1; register the test in the runner list at the end of the file. (2) In `current_script_languages`, capture `exapump`'s status explicitly rather than relying on `errexit`: `output="$(exapump sql -f csv "…" -d "$dsn")" || { echo "error: cannot read the current SCRIPT_LANGUAGES value from $HOST:$PORT" >&2; return 1; }`. (3) In `parse_script_languages`, add a guard after the read loop that returns 1 with `echo "error: the SCRIPT_LANGUAGES query returned no value; refusing to register, which would drop every existing language" >&2` when `line` is empty. (4) At line 744, keep the assignment but make the failure fatal: `EXISTING="$(current_script_languages "$DSN")" || die "cannot read the current SCRIPT_LANGUAGES value; refusing to register"`. Re-run `bash scripts/tests/install-personal-test.sh` and confirm `All assertions passed`.
