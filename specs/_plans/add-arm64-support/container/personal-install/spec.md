# Feature: personal-install

Installs and registers the Rust SLC on an Exasol Personal single-node deployment that exposes no BucketFS HTTP endpoint, so Rust UDFs become available and stay registered across restarts.

## Background

Exasol Personal (local, Apple Silicon) publishes only the SQL port (`8563`) from its VM; it exposes no BucketFS HTTP upload endpoint, so the standard `scripts/install.sh` path (`exapump bucketfs cp` to port `2581`) dead-ends. Personal's Nano engine instead reconciles BucketFS from the VM filesystem: extracting the SLC into `/var/lib/exa/bucketfs/<service>/<bucket>/<slc-name>/` on the VM creates a real bucket within about one second, visible to UDFs at `/buckets/<service>/<bucket>/<slc-name>/`.

The VM is reachable over SSH with the private key at `local/node_access.pem` and an SSH port read from the deployment descriptor `~/.exasol/personal/deployments/<name>/deployment.json` (`connection.sshPort`). The SSH port changes on every `exasol start`, so it must be read fresh on every run and never cached. Registration is a plain `ALTER SYSTEM SET SCRIPT_LANGUAGES` issued over `8563`.

Personal is not exercisable in CI (no arm64 Exasol DB image exists), so the end-to-end scenarios below are verified manually on a live Personal deployment; the string-assembly and entry-preservation logic is unit-tested and is architecture-independent.

## Scenarios

### Scenario: Connection details are read fresh on every run

* *GIVEN* a Personal deployment whose `deployment.json` carries `connection.sshPort` and a key at `local/node_access.pem`
* *WHEN* the Personal install runs
* *THEN* it MUST read the SSH port and key path from `deployment.json` on that run
* *AND* it MUST NOT reuse a cached or previously-recorded SSH port, so it stays correct across an `exasol stop`/`start` cycle that reassigns the port

### Scenario: SLC is deployed via filesystem BucketFS reconciliation

* *GIVEN* a Personal deployment that exposes no BucketFS HTTP endpoint
* *WHEN* the SLC tarball is copied over SSH and extracted into `/var/lib/exa/bucketfs/<service>/<bucket>/<slc-name>/` on the VM
* *THEN* the engine MUST reconcile a real bucket from the filesystem
* *AND* the extracted SLC MUST be visible to UDFs at `/buckets/<service>/<bucket>/<slc-name>/`

### Scenario: Registration targets the exaudfclient executable

* *GIVEN* the SLC extracted under a known bucket path
* *WHEN* the `SCRIPT_LANGUAGES` entry for the `RUST` alias is assembled
* *THEN* its `#` fragment MUST point at the `exaudfclient` executable path, not its containing directory
* *AND* the fragment MUST have no leading slash, because either mistake yields a bare `22002 VM crashed`

### Scenario: Registration is system-scoped and preserves existing entries

* *GIVEN* a Personal database that may already have `SCRIPT_LANGUAGES` entries from `exasol slc install`
* *WHEN* the Personal install registers the `RUST` language
* *THEN* it MUST use `ALTER SYSTEM SET SCRIPT_LANGUAGES` so the registration survives a restart
* *AND* it MUST preserve every pre-existing `SCRIPT_LANGUAGES` entry, adding the `RUST` alias alongside them
* *AND* re-running the install MUST be idempotent across an `exasol stop`/`start` cycle

### Scenario: A registered Rust UDF executes on Personal

* *GIVEN* the SLC installed and the `RUST` language registered on a Personal deployment
* *WHEN* a scalar Rust UDF is created and invoked over `8563`
* *THEN* it MUST return the expected result
* *AND* the registration MUST still resolve after an `exasol stop`/`start` cycle
