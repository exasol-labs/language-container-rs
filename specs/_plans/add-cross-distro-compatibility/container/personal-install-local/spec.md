# Feature: personal-install-local

Installs and registers the Rust SLC on a local Exasol Personal deployment by resolving connection details from the deployment directory and reconciling BucketFS over SSH into the VM filesystem, per the shared dispatch and precedence rules in `container/personal-install`.

## Background

A local Personal deployment publishes only a SQL endpoint; it exposes no BucketFS HTTP upload endpoint, so the standard `exapump bucketfs cp` path (to port `2581`) dead-ends. Personal's Nano engine instead reconciles BucketFS from the node filesystem, as described in `container/personal-install`. The SQL endpoint is a launcher-managed forwarder whose port is assigned per deployment (`exasol config set --ports db:<port>`) and recorded in the descriptor as `connection.dbPort`. `8563` is one deployment's assignment rather than a property of local Personal, so a host running several local deployments serves each on its own port. Local resolution therefore differs from cloud in one field only: its host default is `127.0.0.1`, the address the launcher forwards to. A local descriptor carrying no `connection.dbPort` is malformed, because the launcher always records the assigned port. No BucketFS password is needed, because the local transport never uses the HTTP endpoint. Registration is a plain `ALTER SYSTEM SET SCRIPT_LANGUAGES` issued over the resolved SQL endpoint that preserves every pre-existing entry.

A `.backend` of `local` covers two deployment shapes, and the SLC transport differs between them. One shape runs the database in a managed VM reachable over SSH: the descriptor records `connection.sshPort`, the private key sits at `local/node_access.pem`, and this feature's SSH/filesystem transport applies. The other shape runs the database in a container engine on the invoking host, writes a descriptor with no SSH endpoint, and provides no node key, so no transport this project owns can reach its BucketFS directory. On that shape the supported route is the Personal CLI's own SLC install, which mounts a tarball as a container image instead of copying it into BucketFS. The install distinguishes the two shapes by what the deployment directory provides, not by the host operating system, because the descriptor is the only fact available on every host.

## Scenarios

### Scenario: Local install resolves the DB password from the deployment directory

* *GIVEN* a local Personal deployment whose `secrets.json` carries `.dbPassword`
* *WHEN* the local install runs without `--password`
* *THEN* the DB password MUST come from `secrets.json` `.dbPassword`
* *AND* a `--password` given on the command line MUST override it
* *AND* when neither resolves a password, the local install MUST fail with a clear error

### Scenario: Local connection details resolve from the deployment directory

* *GIVEN* a local Personal deployment whose `deployment.json` carries `connection.dbPort`
* *WHEN* the local install resolves connection details with no overriding command-line flags
* *THEN* the DB host MUST come from `connection.host`, defaulting to `127.0.0.1` when that field is absent, because the launcher forwards the deployment's SQL endpoint to the invoking host
* *AND* the DB port MUST come from `connection.dbPort`, defaulting to `8563` when that field is absent, so on a host running several local deployments the install targets the database named by `--deployment` rather than whichever database answers `8563`
* *AND* the DB user MUST come from `connection.username`, defaulting to `sys` when that field is absent

### Scenario: Command-line flags override descriptor-derived local values

* *GIVEN* a local Personal deployment
* *WHEN* any of `--host`, `--port`, `--user`, or `--password` is given on the command line
* *THEN* each provided flag MUST override the corresponding descriptor-derived value, under the same precedence the cloud path applies
* *AND* any of those values not given on the command line MUST fall back to the descriptor value

### Scenario: A local descriptor that omits the SQL port is reported

* *GIVEN* a local Personal deployment whose `deployment.json` carries no `connection.dbPort`, which the launcher always records
* *WHEN* the local install resolves connection details without `--port`
* *THEN* it MUST warn that the descriptor names no SQL port and that registering over the fallback `8563` risks hitting another local deployment
* *AND* an unreadable `deployment.json` MUST fail with a clear error rather than fall back to a built-in host or port

<!-- DELTA:NEW -->
### Scenario: A local deployment with no SSH transport is reported with the supported route

* *GIVEN* a local Personal deployment whose `deployment.json` carries no `connection.sshPort`, or whose `local/node_access.pem` is absent
* *WHEN* `scripts/install.sh --deployment <name>` runs against it
* *THEN* the install MUST NOT run `scp` or `ssh`, because neither has a reachable endpoint or a usable key on this deployment shape
* *AND* it MUST exit non-zero, naming which of the two facts is missing, so the operator reads the deployment shape rather than a transport error
* *AND* the message MUST name `exasol slc install rust` as the route that installs a published release, and `exasol slc custom install --source <tarball> --language rust` as the route that installs the tarball this run built, and MUST print that tarball's path so the second command needs no rebuild
* *AND* the check MUST run before any tarball is copied, so a deployment this transport cannot serve costs no upload
<!-- /DELTA:NEW -->
