# Feature: personal-install-local

Installs and registers the Rust SLC on a local Exasol Personal deployment (an Apple Silicon VM) by resolving connection details from the deployment directory and reconciling BucketFS over SSH into the VM filesystem, per the shared dispatch and precedence rules in `container/personal-install`.

## Background

A local Personal deployment publishes only a SQL endpoint from its VM; it exposes no BucketFS HTTP upload endpoint, so the standard `exapump bucketfs cp` path (to port `2581`) dead-ends. Personal's Nano engine instead reconciles BucketFS from the VM filesystem, as described in `container/personal-install`. The VM is reachable over SSH with the private key at `local/node_access.pem` and an SSH port read from `deployment.json` (`connection.sshPort`). The SSH port changes on every `exasol start`, so it must be read fresh on every run and never cached. The SQL endpoint is a launcher-managed forwarder whose port is assigned per deployment (`exasol config set --ports db:<port>`) and recorded in the same descriptor as `connection.dbPort`. `8563` is one deployment's assignment rather than a property of local Personal, so a host running several local deployments serves each on its own port. Local resolution therefore differs from cloud in one field only: its host default is `127.0.0.1`, the address the launcher forwards to. A local descriptor carrying no `connection.dbPort` is malformed, because the launcher always records the assigned port. No BucketFS password is needed, because the local transport never uses the HTTP endpoint. Registration is a plain `ALTER SYSTEM SET SCRIPT_LANGUAGES` issued over the resolved SQL endpoint that preserves every pre-existing entry.

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
