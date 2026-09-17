# Decision Log: fix-personal-slc-install

## Interview

**Q:** One plan or two, given issue #110 bundles a container defect and an installer defect?
**A:** One plan, both fixes. It matches the single GitHub issue and comment that raised both.

**Q:** Design direction for the install.sh fix. Delegate to exasol-personal's new `exasol slc custom install` CLI command, or reimplement our own file transfer through the new `vm-shared` mechanism?
**A:** "We could delegate to exasol slc custom install when the version is released, but currently it's not. We need to say old mechanism or personal 2.2, but for recent versions use the new mechanism. We shouldn't compare versions though. We could instead inspect what is available in the deployment directory?"

**Q:** Should install.sh's local path require the new interface only, which breaks older deployments, or support both old and new exasol-personal?
**A:** Support both old and new.

**Q:** After the plan was written it delegated the whole local install to `exasol slc custom install`. Keep that delegation, or have `scripts/install.sh` copy the container into the deployment's shared host directory itself?
**A:** "I would rather not rely on `exasol slc` commands. It was not my intention during the planning." The earlier answer, "We could delegate to exasol slc custom install when the version is released, but currently it's not", had left that open. The user confirmed the direct-copy alternative: `scripts/install.sh` keeps doing its own file transfer and its own `ALTER SYSTEM SET SCRIPT_LANGUAGES` on both local mechanisms, and `exasol slc custom install` is out of scope.

## Design Decisions

### [1] Ship all 21 verified skeleton directories, not the first failing one

- **Decision:** The tarball ships the 21 empty mount-point directories listed in `dist/slc-sandbox-skeleton.txt`, from `boot` to `var/tmp`.
- **Alternatives:** Ship only `proc`, the first reported failure. Rejected because issue #110 records the failure walking from `proc` to `var/tmp` to `buckets`, one directory at a time.
- **Rationale:** The set is the one the issue's follow-up comment verified end to end. It is empirical, not derived from a published contract, so a future host revision may need another entry.
- **Promotes to ADR:** no

### [2] One committed file names the skeleton, read by both the build and the contract test

- **Decision:** `dist/slc-sandbox-skeleton.txt` holds the directory names. The Dockerfile staging stage and `dist/tests/slc_tarball_test.sh` both read it.
- **Alternatives:** Inline the list in the Dockerfile and repeat it in the test. Rejected as back-door leakage: two modules would assume the same list with nothing enforcing agreement.
- **Rationale:** `crates/cargo-exasol-udf/slc-library-surface.txt` already sets this pattern. The skeleton file lives in `dist/` because only the packaging build and its test read it, unlike the library surface, which `cargo exasol-udf validate` also reads.
- **Promotes to ADR:** no

### [3] Skeleton directories ship with default permissions

- **Decision:** The skeleton directories ship with the staging stage's default mode. `/tmp` keeps the `1777` the staging stage already sets for the client's trace files.
- **Alternatives:** Reproduce a full distribution's per-directory modes, for example `1777` on `var/tmp`. Rejected because the verified fix used plain directory creation.
- **Rationale:** The host creates its mount points and mounts over them, so only their presence was observed to matter. Modes added without evidence would be a claim the tests cannot check.
- **Promotes to ADR:** no

### [4] One patch version bump for the whole plan

- **Decision:** `[workspace.package].version` and the pinned `exasol-udf-sdk` entry move to `0.28.1` once, with the regenerated `Cargo.lock` in the same change.
- **Alternatives:** No bump, rejected because the container image changes observably. Two bumps, one per defect, rejected because both ship as one change. A minor bump, rejected because no API changes.
- **Rationale:** The project rule bumps the version for changes downstream users observe, which the container image is. The installer script is tooling and triggers no bump on its own.
- **Promotes to ADR:** no

### [5] Carrying both mechanisms gets a scheduled end

- **Decision:** The plan's PR files a `feature`-labelled GitHub issue to delete the SSH mechanism and its helpers once no supported local Personal deployment publishes SSH inputs, leaving the shared-directory mechanism alone.
- **Alternatives:** Delete the SSH mechanism now, rejected by the user, who asked for both. Carry both with no end date, rejected because no signal would then prompt the removal.
- **Rationale:** Supporting both is a tactical choice taken to avoid breaking existing deployments. The project rule tracks gaps as GitHub issues rather than in a backlog file.
- **Promotes to ADR:** no

### [6] The deployment directory's own contents choose the local install mechanism

- **Decision:** `scripts/install.sh --deployment` takes the SSH mechanism when the deployment directory publishes both SSH inputs, and the shared-directory mechanism when the deployment's BucketFS mapping resolves a host directory for the requested service and bucket. It fails when neither holds, and it reads no version string.
- **Alternatives:** Probe the `exasol` launcher for a custom-SLC subcommand, rejected by the user, who asked for no dependency on `exasol slc` commands. Compare Personal version numbers, rejected by the user at the interview.
- **Rationale:** Each branch reads the literal input its own placement step consumes, so the choice needs no proxy. The SSH mechanism keeps priority where its inputs exist, so no deployment that installs successfully today changes mechanism.
- **Supersedes:** The launcher's own capability chooses the local install mechanism; Detect the launcher's custom-SLC command from its usage output, not its exit status.
- **Promotes to ADR:** yes

### [7] The install script places and registers the container itself on every path

- **Decision:** Both local mechanisms extract the SLC tree themselves and register it with the script's own `ALTER SYSTEM SET SCRIPT_LANGUAGES`. The install script invokes no `exasol` launcher command, so the build keeps its temporary directory and its `EXIT` trap.
- **Alternatives:** Hand the whole local install to the launcher's custom-SLC command, rejected by the user.
- **Rationale:** One placement-and-registration path keeps one entry format, one idempotence rule and one meaning per flag across the local, cloud and cluster installs.
- **Supersedes:** The delegated mechanism hands over placement and registration together; One install command, then verify the outcome the launcher reports; The delegated install hands over a tarball path that outlives the run.
- **Promotes to ADR:** no

### [8] The shared-directory destination comes from the deployment's own BucketFS mapping

- **Decision:** `deployment_bucketfs_dir` reads the deployment's BucketFS mapping file under `local/runtime/vm-shared/exa/`, matches the requested service and bucket, and joins the VM-side path that line names to the shared host directory. It fails for an unmatched pair and for a destination outside the deployment directory.
- **Alternatives:** Hard-code `local/runtime/vm-shared/exa/bucketfs/<service>/<bucket>`, rejected because it restates a mapping the deployment already publishes and accepts a bucket the deployment does not serve.
- **Rationale:** A live check on this workstation confirmed the mapping, the visibility of the extracted tree at `/buckets/bfsdefault/default/<name>/`, and mode `755` on `exaudf/exaudfclient` inside the VM.
- **Promotes to ADR:** no

## Review Findings

### [plan-review] Documentation task named no replacement upload mechanism

- **Finding:** `plan-reviewer` flagged `[AMBIGUOUS_REQUIREMENT]`: task 4.1 replaced the SSH-based UDF-upload snippet with unnamed guidance, which left the plan's only end-to-end check of defect A unrunnable on a current local deployment.
- **Direction change:** Task 4.1 names the host directory under `local/runtime/vm-shared/` that `bucketfs.conf` maps into BucketFS. The Non-Goals bullet is scoped to rebuilding a file transfer inside `scripts/install.sh`.
- **Promotes to ADR:** no

### [plan-review] The removal target's own name component was never validated

- **Finding:** `plan-reviewer` flagged `[NFR_IGNORED]`: only the bucket directory was checked for containment, so a `--slc-name` of `../../..` walked out of the bucket subtree and `rm -rf` reached the deployment's `exa/storage/vol_*.dat` volumes.
- **Direction change:** Task 2.2 adds `require_path_segment`, rejecting `/`, `.`, `..`, a leading `-` and an empty value for `--bfs-service`, `--bucket` and `--slc-name`. Task 2.5 also refuses any destination that is not a physically-resolved direct child of the bucket directory. The spec scenario requires both checks.
- **Promotes to ADR:** no

### [plan-review] "Removes it" read as removing the whole bucket directory

- **Finding:** `plan-reviewer` flagged `[COMPLETENESS_GAP]`: task 2.4 resolved the destination through `deployment_bucketfs_dir` and then "removes it", which reads as removing the bucket directory the plan's own docs task tells operators to keep their `udf/*.so` files in.
- **Direction change:** Task 2.5 names the joined `<bucket dir>/<slc-name>` path as the only thing removed. Task 2.4's test seeds a sibling `udf/libother.so` and asserts it survives both extractions. The spec scenario requires every file outside `<slc-name>/` to stay untouched.
- **Promotes to ADR:** no

### [plan-review] Status 1 conflated an unusable directory with a wrong flag

- **Finding:** `plan-reviewer` flagged `[REQUIREMENT_CONFLICT]`: three causes sat under exit status 1, and task 3.3 reported all of them as "the requested pair is not served". An operator whose BucketFS directory does not exist yet would be told their `--bucket`/`--bfs-service` flags are wrong.
- **Direction change:** `deployment_bucketfs_dir` returns three statuses: 2 for an absent mapping file, 3 for an unmatched pair, 1 for an unusable resolved host directory. Task 3.3 branches on all three and names the directory rather than the flags on status 1. Task 2.1 asserts the status of each failure fixture, and the selection scenario gained the matching clause.
- **Promotes to ADR:** no
