# Feature: personal-install

Installs and registers the Rust SLC on an Exasol Personal single-node deployment — local (SSH into the VM, filesystem BucketFS) or cloud (BucketFS HTTP) — so Rust UDFs become available on Personal.

## Background

An Exasol Personal deployment runs its single-node database either locally (an Apple Silicon VM) or on a cloud backend (`aws`, `azure`, `exoscale`, `stackit`). `scripts/install.sh --deployment <name>` reads the deployment descriptor `~/.exasol/personal/deployments/<name>/deployment.json` and branches on its `.backend` field, mirroring the Personal launcher's own `IsLocalBackend()`: a `.backend` of `local` selects the SSH/filesystem transport; any other value selects the standard BucketFS HTTP transport. A missing or empty `.backend` is a malformed descriptor and fails the run.

A local Personal deployment publishes only the SQL port (`8563`) from its VM; it exposes no BucketFS HTTP upload endpoint, so the standard `exapump bucketfs cp` path (to port `2581`) dead-ends. Personal's Nano engine instead reconciles BucketFS from the VM filesystem: extracting the SLC into `/var/lib/exa/bucketfs/<service>/<bucket>/<slc-name>/` on the VM creates a real bucket within about one second, visible to UDFs at `/buckets/<service>/<bucket>/<slc-name>/`. The VM is reachable over SSH with the private key at `local/node_access.pem` and an SSH port read from `deployment.json` (`connection.sshPort`). The SSH port changes on every `exasol start`, so it must be read fresh on every run and never cached. The DB password is read from the sibling `secrets.json` `.dbPassword`, with a command-line `--password` overriding it when given; no BucketFS password is needed, because the local transport never uses the HTTP endpoint. Registration is a plain `ALTER SYSTEM SET SCRIPT_LANGUAGES` issued over `8563` that preserves every pre-existing entry.

A cloud Personal deployment reaches the database over the network and exposes the BucketFS HTTP endpoint (port `2581`, with the `bfsdefault/default` bucket auto-created), so it is the standard HTTP transport with connection details harvested from the deployment directory rather than typed on the command line. `deployment.json` carries `connection.host`, `connection.dbPort` (an integer; `8563` when absent), and `connection.username` (`sys` when absent); the DB password lives in the sibling `secrets.json` `.dbPassword`. Personal provisions no BucketFS read/write password anywhere, so the operator MUST supply `--bfs-password`. Any of `--host`, `--port`, `--user`, `--password` given on the command line overrides the descriptor-derived value. After resolving connection details, the cloud path runs the same upload-and-register steps as a non-Personal install with no behavioral change.

Personal is not exercisable in CI (no arm64 Exasol DB image exists, and this workflow does not reach a live cloud deployment), so the end-to-end scenarios below are verified manually on a live Personal deployment; the descriptor-parsing, connection-resolution, and string-assembly logic is unit-tested and is architecture-independent.

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

### Scenario: Local install resolves the DB password from the deployment directory

* *GIVEN* a local Personal deployment whose `secrets.json` carries `.dbPassword`
* *WHEN* the local install runs without `--password`
* *THEN* the DB password MUST come from `secrets.json` `.dbPassword`
* *AND* a `--password` given on the command line MUST override it
* *AND* when neither resolves a password, the local install MUST fail with a clear error

### Scenario: Deployment backend selects the transport

* *GIVEN* a Personal deployment whose `deployment.json` carries a `.backend` field
* *WHEN* `scripts/install.sh --deployment <name>` runs
* *THEN* it MUST read `.backend` from `deployment.json` on that run
* *AND* a `.backend` of `local` MUST select the SSH/filesystem transport described by the local scenarios
* *AND* any other `.backend` value MUST select the standard BucketFS HTTP transport
* *AND* a missing or empty `.backend` MUST fail with a clear error rather than assume a transport

### Scenario: Cloud connection details resolve from the deployment directory

* *GIVEN* a cloud Personal deployment whose `deployment.json` carries `connection.host`, `connection.dbPort`, and `connection.username`, and whose `secrets.json` carries `.dbPassword`
* *WHEN* the cloud install resolves connection details with no overriding command-line flags
* *THEN* the DB host MUST come from `connection.host`
* *AND* the DB port MUST come from `connection.dbPort`, defaulting to `8563` when the field is absent
* *AND* the DB user MUST come from `connection.username`, defaulting to `sys` when the field is absent
* *AND* the DB password MUST come from `secrets.json` `.dbPassword`

### Scenario: Command-line flags override descriptor-derived cloud values

* *GIVEN* a cloud Personal deployment
* *WHEN* any of `--host`, `--port`, `--user`, or `--password` is given on the command line
* *THEN* each provided flag MUST override the corresponding descriptor-derived value
* *AND* any of those values not given on the command line MUST fall back to the descriptor value

### Scenario: Cloud install requires an operator-supplied BucketFS password

* *GIVEN* a cloud Personal deployment, which provisions no BucketFS read/write password
* *WHEN* the cloud install runs without `--bfs-password`
* *THEN* it MUST fail with a clear error stating that Personal provisions no BucketFS password and that `--bfs-password` is required
* *AND* when `--bfs-password` is supplied, the cloud install MUST proceed

### Scenario: Cloud install fails when no DB password resolves

* *GIVEN* a cloud Personal deployment whose `secrets.json` is absent, or whose `secrets.json` `.dbPassword` is empty
* *AND* no `--password` is given on the command line
* *WHEN* the cloud install resolves connection details
* *THEN* it MUST fail with a clear error stating that no DB password resolved
* *AND* when `--password` is given, the cloud install MUST use it as the DB password and proceed

### Scenario: Cloud install uses the standard HTTP transport and scope

* *GIVEN* a cloud Personal deployment with resolved connection details and `--bfs-password`
* *WHEN* the cloud install transports and registers the SLC
* *THEN* it MUST upload the tarball over the BucketFS HTTP API (`exapump bucketfs cp`), exactly as the non-Personal path does
* *AND* it MUST register with `ALTER <scope>` honoring `--scope`, whose default is `SESSION`
* *AND* it MUST NOT force `SYSTEM` scope and MUST NOT apply the read-merge-write entry preservation used by the local path
