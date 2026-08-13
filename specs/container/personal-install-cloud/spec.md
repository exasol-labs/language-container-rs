# Feature: personal-install-cloud

Installs and registers the Rust SLC on a cloud Exasol Personal deployment (`aws`, `azure`, `exoscale`, `stackit`) by resolving connection details from the deployment directory and uploading over the standard BucketFS HTTP transport, per the shared dispatch and precedence rules in `container/personal-install`.

## Background

A cloud Personal deployment reaches the database over the network and exposes the BucketFS HTTP endpoint (port `2581`, with the `bfsdefault/default` bucket auto-created), so it is the standard HTTP transport with connection details harvested from the deployment directory rather than typed on the command line. Cloud has no host default: `deployment.json` MUST carry `connection.host` or the operator MUST pass `--host`. Personal provisions no BucketFS read/write password anywhere, so the operator MUST supply `--bfs-password`; that credential is not a connection field, and only the cloud path requires it. After resolving connection details, the cloud path runs the same upload-and-register steps as a non-Personal install with no behavioral change.

## Scenarios

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
