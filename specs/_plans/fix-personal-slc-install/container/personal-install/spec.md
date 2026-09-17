# Feature: personal-install

<!-- DELTA:CHANGED -->
Installs and registers the Rust SLC on an Exasol Personal single-node deployment, local or cloud, so Rust UDFs become available on Personal.
<!-- /DELTA:CHANGED -->

## Background

<!-- DELTA:CHANGED -->
An Exasol Personal deployment runs its single-node database either locally (a VM or a container on this machine) or on a cloud backend (`aws`, `azure`, `exoscale`, `stackit`). `scripts/install.sh --deployment <name>` reads the deployment descriptor `~/.exasol/personal/deployments/<name>/deployment.json` and branches on its `.backend` field, mirroring the Personal launcher's own `IsLocalBackend()`: a `.backend` of `local` selects the local transport (`container/personal-install-local`); any other value selects the standard BucketFS HTTP transport (`container/personal-install-cloud`). A missing or empty `.backend` is a malformed descriptor and fails the run.

Every transport resolves the same four connection fields from the same deployment directory, under one precedence rule. A command-line `--host`, `--port`, `--user`, or `--password` wins. Otherwise the value comes from `deployment.json`'s `.connection.host`, `.connection.dbPort`, or `.connection.username`, or from `secrets.json`'s `.dbPassword`. The port then defaults to `8563` and the user to `sys`. The host default is the only field that differs between the backends; `container/personal-install-local` and `container/personal-install-cloud` each specify their own resolution scenarios. An unresolved host or DB password fails the run, and each transport prints the resolved endpoint before registering.
<!-- /DELTA:CHANGED -->

Personal is not exercisable in CI (no arm64 Exasol DB image exists, and this workflow does not reach a live cloud deployment), so the end-to-end scenarios below, and those in `container/personal-install-local` and `container/personal-install-cloud`, are verified manually on a live Personal deployment; the descriptor-parsing, connection-resolution, and string-assembly logic is unit-tested and is architecture-independent.

<!-- DELTA:NEW -->
A local deployment install uses one of two mechanisms, and both end at the same place. The SSH mechanism copies the tarball into the VM over SSH, using an SSH port and a node key that the deployment directory publishes. The shared-directory mechanism extracts the tarball into a host directory that the deployment itself shares into the VM. The deployment directory's own contents choose the mechanism, and the install script reads no Personal version string.

The two mechanisms differ in the placement step alone. Both leave the same SLC tree at the same bucket path. Both then wait for the engine to reconcile the bucket and register the language with the install script's own `ALTER SYSTEM SET SCRIPT_LANGUAGES`. Both therefore resolve the same four connection fields, and every BucketFS placement option applies to both.
<!-- /DELTA:NEW -->

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Connection details are read fresh on every run

* *GIVEN* a Personal deployment whose `deployment.json` carries `connection.sshPort` and `connection.dbPort`, and a key at `local/node_access.pem`, which selects the SSH mechanism
* *WHEN* the Personal install runs
* *THEN* it MUST read the SSH port, the SQL port, and the key path from `deployment.json` on that run
* *AND* it MUST NOT reuse a cached or previously-recorded value for either port, so both stay correct after an `exasol stop`/`start` cycle that reassigns the SSH port and after an `exasol config set --ports db:<port>` that reassigns the SQL port
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: SLC is deployed via filesystem BucketFS reconciliation

* *GIVEN* a Personal deployment that exposes no BucketFS HTTP endpoint
* *WHEN* the install places the SLC tree at the deployment's own BucketFS filesystem location for `<service>/<bucket>/<slc-name>`, either over SSH into the VM or by extracting into the host directory the deployment shares into the VM
* *THEN* the engine MUST reconcile a real bucket from the filesystem
* *AND* the extracted SLC MUST be visible to UDFs at `/buckets/<service>/<bucket>/<slc-name>/`
* *AND* `exaudf/exaudfclient` MUST be readable and executable at that path, whichever mechanism wrote the tree
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: Registration targets the exaudfclient executable

* *GIVEN* the SLC extracted under a known bucket path, over either local mechanism or the BucketFS HTTP transport
* *WHEN* the `SCRIPT_LANGUAGES` entry for the `RUST` alias is assembled
* *THEN* its `#` fragment MUST point at the `exaudfclient` executable path, not its containing directory
* *AND* the fragment MUST have no leading slash, because either mistake yields a bare `22002 VM crashed`
* *AND* the entry MUST be assembled from the same `--bfs-service`, `--bucket` and `--slc-name` values on both local mechanisms, so the mechanism never reaches the registration string
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: Registration is system-scoped and preserves existing entries

* *GIVEN* a Personal database that may already have `SCRIPT_LANGUAGES` entries for its built-in languages, and a local deployment on either mechanism
* *WHEN* the Personal install registers the `RUST` language
* *THEN* it MUST use `ALTER SYSTEM SET SCRIPT_LANGUAGES` so the registration survives a restart
* *AND* it MUST print the resolved `host:port` it is registering against before issuing the statement, so a wrong target is visible without querying the database
* *AND* it MUST preserve every pre-existing `SCRIPT_LANGUAGES` entry, adding the `RUST` alias alongside them and replacing an earlier `RUST` entry rather than adding a second one
* *AND* re-running the install MUST be idempotent across an `exasol stop`/`start` cycle
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: A registered Rust UDF executes on Personal

* *GIVEN* the SLC installed and the `RUST` language registered on a Personal deployment, by either local mechanism or by the cloud transport
* *WHEN* a scalar Rust UDF is created and invoked over that deployment's SQL port
* *THEN* it MUST return the expected result
* *AND* the registration MUST still resolve after an `exasol stop`/`start` cycle
<!-- /DELTA:CHANGED -->

<!-- DELTA:CHANGED -->
### Scenario: Deployment backend selects the transport

* *GIVEN* a Personal deployment whose `deployment.json` carries a `.backend` field
* *WHEN* `scripts/install.sh --deployment <name>` runs
* *THEN* it MUST read `.backend` from `deployment.json` on that run
* *AND* a `.backend` of `local` MUST select the local transport described in `container/personal-install-local`, which then chooses between its two placement mechanisms
* *AND* any other `.backend` value MUST select the standard BucketFS HTTP transport described in `container/personal-install-cloud`
* *AND* a missing or empty `.backend` MUST fail with a clear error rather than assume a transport
<!-- /DELTA:CHANGED -->
