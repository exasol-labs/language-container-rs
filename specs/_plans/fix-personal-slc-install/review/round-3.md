# Plan Review Findings: fix-personal-slc-install (round 3)

## Summary
- Axes checked: 6/6
- Total findings: 14 (Blockers: 3, Advisory: 11)
- Intent Fidelity blockers: 0
- Mode: full fresh pass. Defect B was redesigned after the user reversed the CLI-delegation choice, so this round re-reviews it from scratch. Round-1 and round-2 blockers targeted the superseded delegated design and are not rechecked here.

## Premortem

Six months out, three ways this plan failed.

1. An operator ran `scripts/install.sh --deployment default` a second time and lost data. The shared-directory placement removed more than the `<slc-name>` tree: either the whole bucket directory (taking the `udf/*.so` artifacts that `docs/installation.md` itself tells operators to put there), or, with a `--slc-name` carrying `..`, the deployment's `exa/` directory including `exa/storage/vol_*.dat`. The specified guard checked the bucket directory, never the path actually removed. → `[COMPLETENESS_GAP]`, `[NFR_IGNORED]`.
2. An operator typed `--bucket defualt`. The run failed with "provide an SSH port and node key, or a BucketFS mapping", so the operator concluded the tool did not support their Personal release and stopped. The mapping was present and one flag was wrong. → `[REQUIREMENT_CONFLICT]`.
3. The install reported success, a UDF ran, and after `exasol stop && exasol start` every call returned `22002` again. The launcher re-applied its own `RUST=` entry for the custom SLC its `slc-status.json` still records, overwriting the script's registration. Nothing in the plan states that the launcher does not do this. → `[UNSTATED_ASSUMPTION]`.

## Intent Fidelity

[no objection — axis checked: no live dependency on the `exasol` CLI remains. A grep of `plan.md`, `decision-log.md` and the three deltas for `exasol slc`, `custom install` and `launcher` returns only the verbatim interview quotes, the superseded decisions [1]-[4] and [10] carrying explicit `Status: Superseded` lines, the rejected-alternative columns, the `Non-Goals` bullet, and prose that names the launcher as the process that runs the VM and assigns the SQL port. "Support both old and new" holds: the selection rule, the three-fixture selector test, the `container/personal-install-local` mechanism scenarios and two Manual Testing rows all carry the SSH mechanism. "No version comparison" holds and is stated normatively in the delta.]

## Feasibility

#### [NFR_IGNORED] BLOCKER
- Location: plan.md § Implementation Tasks 2.2 and 2.4; `container/personal-install-local` § "The shared-directory destination is checked before anything is removed"
- Issue: the host-side `rm -rf` target is `<resolved bucket directory>/<slc-name>`, but nothing validates the `<slc-name>` component. `scripts/install.sh:414` checks only `[[ -z "$SLC_NAME" ]]`. Task 2.2 gives `deployment_bucketfs_dir <dir> <service> <bucket>` a two-argument signature, so its containment check covers the bucket directory alone, and the boundary it checks is "not inside `<dir>`" — the whole deployment directory. The spec scenario's WHEN clause names three inputs ("resolves the destination for the given `--bfs-service`, `--bucket` and `--slc-name`") while its THEN clause checks only "the resolved destination falls outside the deployment directory". On the live deployment the bucket directory is `~/.exasol/personal/deployments/default/local/runtime/vm-shared/exa/bucketfs/bfsdefault/default`, so `--slc-name ../../..` resolves to `.../vm-shared/exa`, passes the "inside the deployment directory" check, and `rm -rf` destroys `exa/storage/vol_1000.dat` and `vol_1008.dat` — the database's data volumes. The plan's own Patterns table claims this is guarded: "Resolve and check the destination before removing it … A path outside the deployment directory is a bug to refuse, not to perform."
- Fix: In plan.md § Implementation Tasks, add a task before 2.2 that rejects a `--slc-name`, `--bucket` or `--bfs-service` value containing `/`, equal to `.` or `..`, or starting with `-`, at the existing non-empty validation block in `scripts/install.sh`, and add its assertions to task 2.1's test list. Change task 2.4 to state that `extract_slc_into_shared_bucketfs` refuses any destination that is not a direct child of the directory `deployment_bucketfs_dir` returned, and that both the bucket directory and the final destination are compared after physical resolution (`cd … && pwd -P`, or `realpath`), not by string prefix on the unresolved join. In `container/personal-install-local` § "The shared-directory destination is checked before anything is removed", replace the clause "it MUST fail when the resolved destination falls outside the deployment directory" with "it MUST fail when the destination it removes is not a direct child of the host directory the mapping resolved, and MUST fail when that host directory itself resolves outside the deployment directory", and add a clause requiring `--slc-name` to be a single path segment.

#### [UNSTATED_ASSUMPTION] ADVISORY
- Location: plan.md § Implementation Tasks 2.4; § Dependencies
- Issue: the SSH mechanism extracts inside the VM with the VM's GNU `tar`. The shared-directory mechanism extracts with the invoking host's `tar`, which on the documented macOS workstation is bsdtar. The only post-extraction check specified is "confirms `exaudf/exaudfclient` is executable". `container/slim-image` requires `etc/hosts` and `etc/resolv.conf` to be symbolic links into `/conf/` for DNS to work, and requires the usr-merge symlinks to resolve. A tar that drops or rewrites those links leaves an SLC that passes the specified check and fails at run time. The planner's live probe extracted a launcher-built `custom-rust-*.tar.gz` and never ran a Rust UDF from the extracted tree.
- Fix: Extend task 2.4 in plan.md to require `extract_slc_into_shared_bucketfs` to also confirm that `etc/hosts` and `etc/resolv.conf` are symbolic links after extraction, and extend task 2.3's test to build a fixture tarball carrying one symbolic link and assert it survives. Add a sentence to plan.md § Dependencies naming the host `tar` implementations the mechanism is verified against.

#### [UNSTATED_ASSUMPTION] ADVISORY
- Location: plan.md § Impact; `container/personal-install` § "Registration is system-scoped and preserves existing entries"
- Issue: the live deployment already carries `RUST=/__builtin__/slc/custom-rust` in `SCRIPT_LANGUAGES`, and its `local/runtime/vm-shared/slc-status.json` still records `exasol-personal/custom-slc:rust-…` with `"state":"imported"`. The install replaces that entry. The plan never states whether the launcher re-applies its recorded entry on the next start. The recorded scenario requires "the registration MUST still resolve after an `exasol stop`/`start` cycle" and "re-running the install MUST be idempotent across an `exasol stop`/`start` cycle", so the claim depends on an unverified behaviour of a component this plan does not control.
- Fix: Add one row to plan.md § Manual Testing: run the shared-directory install, then `exasol stop && exasol start`, then re-query `SCRIPT_LANGUAGES`, expecting the script's own `RUST=` entry naming `/buckets/bfsdefault/default/rustslc/`. Add a sentence to plan.md § Impact stating that the install replaces a launcher-recorded `RUST=` entry, so the launcher's container list and the database's registration then disagree.

#### [UNSTATED_ASSUMPTION] ADVISORY
- Location: plan.md § Manual Testing, the last two rows; § Verification / Scenario Coverage, "A registered Rust UDF executes on Personal"
- Issue: two Manual Testing rows and one Scenario Coverage row require a deployment directory that publishes `.connection.sshPort` and `local/node_access.pem`. `notes/planning.md` records that the current launcher publishes neither and that "the local VM forwards only the database port now. No SSH forward exists". The plan states the SSH result as fact ("unchanged from today") without saying how that evidence is produced, while task 3.5 moves the SSH port and key reads into a new branch.
- Fix: Add one sentence under plan.md § Manual Testing naming what produces an SSH-capable fixture (an older launcher release, or a hand-built deployment directory), or mark those two rows as regression-tested by the unit fixtures alone and state that no live SSH run is available.

#### [NFR_IGNORED] ADVISORY
- Location: plan.md § Implementation Tasks 2.4
- Issue: the step order is remove, recreate, extract, check. A failure between the remove and the check leaves the bucket with the previous working SLC gone and no replacement, on the operator's own machine. The plan states no recovery instruction.
- Fix: Add to plan.md § Impact one sentence naming the failure mode: a failed extraction leaves the `<slc-name>` directory incomplete, and the operator recovers by re-running the install.

## Requirement Quality

#### [COMPLETENESS_GAP] BLOCKER
- Location: plan.md § Implementation Tasks 2.3 and 2.4; `container/personal-install-local` § "The shared-directory mechanism extracts into the deployment's own BucketFS directory"
- Issue: task 2.4 reads "resolves the destination through `deployment_bucketfs_dir`, prints it, removes it, recreates it, extracts the tarball into it". `deployment_bucketfs_dir` returns the bucket directory, so "removes it" reads as removing the bucket directory itself. The spec clause "it MUST replace `<slc-name>/` under that host directory" says otherwise, and the spec's only containment clause is "write nothing outside that host directory", which permits removing everything inside it. Task 2.3's test asserts only "the stale file is gone, the tree is present once" for a stale file placed under `<slc-name>/`, so it passes either way. The live deployment holds `local/runtime/vm-shared/exa/bucketfs/bfsdefault/default/udf/libscalar_double.so`, and task 4.1 of this same plan instructs operators to put UDF `.so` files at exactly that path. The plan therefore documents a location that a correct-looking reading of its own placement task deletes.
- Fix: Rewrite task 2.4 in plan.md to read: resolves the bucket directory through `deployment_bucketfs_dir`, joins `<slc-name>` to it, prints that joined path, removes only that path, recreates it, extracts into it. Extend task 2.3's test to seed the fixture bucket directory with a sibling file outside `<slc-name>/` (for example `udf/libother.so`) and assert it is still present after both extractions. Add a clause to the `container/personal-install-local` scenario "The shared-directory mechanism extracts into the deployment's own BucketFS directory": "*AND* it MUST leave every file under that host directory outside `<slc-name>/` untouched, because the same bucket carries the operator's own UDF artifacts".

#### [REQUIREMENT_CONFLICT] BLOCKER
- Location: plan.md § Implementation Tasks 3.3 against plan.md § Manual Testing row 6 and `container/personal-install-local` § "The shared-directory destination is checked before anything is removed"
- Issue: task 3.3 specifies one failure message: `personal_local_mechanism` "otherwise fails with one error naming both mechanisms' prerequisites". plan.md § Manual Testing expects the opposite for `scripts/install.sh --deployment default --bucket nosuchbucket`: "The run fails naming the service and bucket pair the deployment's BucketFS mapping does not serve". The spec scenario's first THEN clause likewise requires the unmatched-pair case to be distinguishable ("because extraction there would create a directory the engine never reconciles"). As specified, a typo in `--bucket` on a current deployment produces a generic message telling the operator to supply SSH inputs, which points at the deployment rather than the flag. Task 3.1's selector test covers only "neither present with no mapping file", so no test catches the collapse.
- Fix: Change task 3.3 in plan.md so `personal_local_mechanism` distinguishes two shared-directory failures: when the mapping file is absent it reports both mechanisms' prerequisites, and when the mapping file is present but names no line for the requested service and bucket it reports that pair and the pairs the mapping does serve. Add a fourth fixture to task 3.1's test covering a present mapping file with an unmatched pair, asserting the pair-specific message.

#### [AMBIGUOUS_REQUIREMENT] ADVISORY
- Location: plan.md § Implementation Tasks 2.2; `container/personal-install-local` § Background, paragraph 3
- Issue: both artifacts say "the VM-side path that line names", singular. A live mapping line names two paths: `/exa/bucketfs/bfsdefault/default bfsdefault default /buckets/bfsdefault/default - P`. Field 1 is the VM-side host path that joins to `vm-shared`; field 4 is the bucket path the engine serves. Joining field 4 produces a path that does not exist. The `__builtin__` line shows the two are not interchangeable: `/exa/slc __builtin__ slc /exa/slc dDE= P` publishes a field 4 that is not a `/buckets/…` path at all, so for a non-default `--bfs-service`/`--bucket` the placement step and the `/buckets/<service>/<bucket>/<slc-name>/` string that `script_languages_entry` builds can name different locations.
- Fix: In plan.md task 2.2, replace "the VM-side path that line names" with the field position and its meaning: the first whitespace-separated field, the VM-side directory that backs the bucket, matched on fields 2 and 3. Add a failure condition: the function fails when the line's fourth field is not `/buckets/<service>/<bucket>`, because the registration string assumes that path. Mirror both in the `container/personal-install-local` Background paragraph and add the mismatch case to task 2.1's test list.

#### [COMPLETENESS_GAP] ADVISORY
- Location: plan.md § Implementation Tasks 3.2
- Issue: `deployment_supports_ssh_transport` "Reuse[s] `deployment_ssh_port`", which writes `error: <descriptor> carries no numeric connection.sshPort` to stderr before returning 1 (`scripts/install.sh:164`). On a current deployment that is the normal, successful shared-directory path, so every install prints a line beginning with `error:` and then succeeds.
- Fix: State in plan.md task 3.2 that `deployment_supports_ssh_transport` suppresses the accessor's stderr, and add an assertion to task 3.1's test that selecting the shared mechanism writes no line matching `^error:`.

## Task Breakdown

#### [TRACEABILITY_GAP] ADVISORY
- Location: plan.md § Verification / Scenario Coverage, "Both local mechanisms register through the install script"; § Implementation Tasks 3.4
- Issue: the table claims Integration coverage through `local_mechanisms_share_the_registration_inputs`. Task 3.4 defines that test as asserting "both yield the same resolved endpoint from `resolve_deployment_connection` and the same string from `script_languages_entry`". Neither function takes the mechanism as an input, so the test cannot fail for the risk the scenario names: that the rewired `main` in task 3.5 skips the reconciliation wait or the `ALTER SYSTEM` on one branch. The harness drives functions in-process and never drives `main` (`scripts/tests/install-personal-test.sh`, `run()` at line 464), so no automated test reaches the dispatch.
- Fix: Change task 3.5 in plan.md to extract the local placement-and-register sequence into one function that takes the mechanism name and calls an injectable placement command, then redefine task 3.4's test to drive that function once per mechanism with a recording stub and assert both runs reach the reconciliation wait and the `ALTER SYSTEM` step. If that refactor is out of scope, change the Scenario Coverage row's Test Type to Manual and name the Manual Testing rows that carry it.

#### [TRACEABILITY_GAP] ADVISORY
- Location: plan.md § Dependencies; § Implementation Tasks 3.6
- Issue: "The shared-directory mechanism adds one host command, `tar`, to the local install path, which already needs `jq` and `exapump`." Task 3.6 updates only `usage()` and the `--deployment` help text. No task adds a `require_command tar` call, and `scripts/install.sh` has no `require_command ssh` or `require_command scp` today either. A host without `tar` reaches the placement step and fails after the build, not at validation.
- Fix: Add to plan.md task 3.5 a clause requiring the dispatch to call `require_command tar` on the `shared` branch and `require_command ssh` plus `require_command scp` on the `ssh` branch, before the placement step runs.

#### [TRACEABILITY_GAP] ADVISORY
- Location: plan.md § Implementation Tasks 4.1
- Issue: task 4.1 scopes the documentation edit to "`docs/installation.md`'s Exasol Personal section", which starts at line 55. Line 10's transport table states the container "travels over SSH into the VM's BucketFS directory instead of being uploaded" as the only local behaviour, and line 104's callout heading introduces the SSH-only UDF-upload snippet. Both become wrong once two mechanisms exist, and line 10 falls outside the task's stated scope.
- Fix: Extend plan.md task 4.1 to name `docs/installation.md` line 10's transport table row as well as the Personal section, and state the corrected one-line description for that row.

## Design Depth

#### [INFORMATION_LEAKAGE] ADVISORY
- Location: plan.md § Design / Architecture and § Patterns; Implementation Tasks 2.2 and 2.4
- Issue: one decision, "which host path this install removes and extracts into", is split across two functions. `deployment_bucketfs_dir` owns the mapping lookup and the safety checks. `extract_slc_into_shared_bucketfs` owns the `<slc-name>` join. Nothing enforces agreement, which is what makes the unchecked `<slc-name>` component reachable (see the Feasibility BLOCKER). The plan's own text asserts single ownership: "`deployment_bucketfs_dir` owns the destination."
- Fix: In plan.md task 2.2, change `deployment_bucketfs_dir` to take the SLC name as a third argument and return the full destination path it has already checked, so `extract_slc_into_shared_bucketfs` joins nothing. Update the § Design / Architecture diagram caption and the § Patterns row "Resolve and check the destination before removing it" to name the returned value as the exact path removed.

## Prose Quality

#### [PROSE_BLOAT] ADVISORY
- Location: plan.md § Summary line 5, § Design / Context lines 13 and 17
- Issue: three descriptive sentences exceed the 25-word cap and each states more than one idea. Line 5: "Teach `scripts/install.sh --deployment` a second way to place the container on a local deployment, by extracting it into the host directory the deployment shares into its VM, and keep the SSH transport for a deployment that still publishes SSH inputs." (41 words, joined by "and"). Line 13: "Its sandbox setup creates its mount points with `mkdir`, which succeeds as a no-op against a full-distribution container such as the official Python3 SLC and fails against ours." (29 words, joined by "which" and "and"; "ours" also leaves the actor unnamed). Line 17: "A live check on this workstation extracted an SLC tarball into the host directory that mapping names for `bfsdefault`/`default`, and a Python UDF on the running deployment then listed the tree at `/buckets/bfsdefault/default/<name>/` and reported `exaudf/exaudfclient` as mode `755`, readable and executable." (44 words, two "and" joins).
- Fix: Split each of the three sentences in plan.md into two, one idea per sentence, each under 25 words, and replace "ours" on line 13 with "the Rust SLC".
