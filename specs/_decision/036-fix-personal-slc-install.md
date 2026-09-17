# Decisions: fix-personal-slc-install

## ADR: The deployment directory's own contents choose the local install mechanism

**ID:** local-install-mechanism-from-deployment-directory
**Plan:** fix-personal-slc-install
**Status:** Accepted

### Context

`scripts/install.sh --deployment` places the SLC on a local Exasol Personal deployment through one of two mechanisms: SSH into the VM, or extraction into a host directory the deployment shares into the VM. A current local deployment publishes no SSH port and no node key, so the SSH mechanism alone no longer covers every supported deployment. The install must pick the right mechanism without depending on the `exasol` launcher or on a Personal version number.

### Decision

`scripts/install.sh --deployment` takes the SSH mechanism when the deployment directory publishes both an SSH port and a readable node key. It takes the shared-directory mechanism when the deployment's BucketFS mapping resolves a host directory for the requested service and bucket. It fails when neither holds, and it reads no version string.

### Options Considered

| Option | Verdict |
|--------|---------|
| Read the deployment directory's own inputs for each mechanism | ✓ Chosen — each branch reads the literal input its own placement step consumes, so the choice needs no proxy |
| Detect the launcher's custom-SLC command from its usage output | ✗ Rejected — the user asked for no dependency on `exasol slc` commands |
| Compare Personal version numbers | ✗ Rejected by the user at the interview |

### Consequences

A deployment that installs successfully today keeps its mechanism, because the SSH mechanism keeps priority wherever its inputs are present. A deployment that publishes neither mechanism's inputs fails with one error naming both mechanisms' prerequisites, instead of a mid-run failure that only reads as an SSH problem.
