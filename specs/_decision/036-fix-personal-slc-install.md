# Decisions: fix-personal-slc-install

## ADR: The deployment directory's own contents choose the local install mechanism

**ID:** local-install-mechanism-from-deployment-directory
**Plan:** fix-personal-slc-install
**Status:** Accepted

### Context

`scripts/install.sh --deployment` places the SLC on a local Exasol Personal deployment by SSH into the VM or by extraction into a host directory shared into the VM. Some deployments publish no SSH port and no node key. The install must choose without depending on the `exasol` launcher or a Personal version number.

### Decision

`scripts/install.sh --deployment` uses SSH when the deployment directory publishes both an SSH port and a readable node key. It uses the shared directory when the deployment's BucketFS mapping resolves a host directory for the requested service and bucket. It fails when neither holds and reads no version string.

### Options Considered

| Option | Verdict |
|--------|---------|
| Read the deployment directory's own inputs per mechanism | ✓ Chosen |
| Detect the launcher's custom-SLC command from usage output | ✗ The install must not depend on `exasol slc` commands |
| Compare Personal version numbers | ✗ The install must not depend on a version number |

### Consequences

SSH keeps priority wherever its inputs exist. A deployment with neither mechanism's inputs fails with one error naming both prerequisites.
