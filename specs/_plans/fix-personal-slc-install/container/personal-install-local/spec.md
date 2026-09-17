# Feature: personal-install-local

<!-- DELTA:CHANGED -->
Installs and registers the Rust SLC on a local Exasol Personal deployment by placing the container where the engine reconciles BucketFS from the filesystem, over SSH into the VM or by extracting into the host directory the deployment shares into the VM, and then registering it per the shared dispatch and precedence rules in `container/personal-install`.
<!-- /DELTA:CHANGED -->

## Background

<!-- DELTA:CHANGED -->
A local Personal deployment publishes only a SQL endpoint from its VM. It exposes no BucketFS HTTP upload endpoint, so the standard `exapump bucketfs cp` path (to port `2581`) dead-ends. Personal's Nano engine reconciles BucketFS from the filesystem instead, as described in `container/personal-install`. The local install therefore places the SLC tree in that filesystem location and registers the language itself. Two mechanisms reach that location, and the deployment directory's own contents choose between them.

The SSH mechanism reaches the VM over SSH, with the private key at `local/node_access.pem` and an SSH port read from `deployment.json` (`connection.sshPort`). The SSH port changes on every `exasol start`, so the install reads it fresh on every run and caches nothing. The install copies the tarball in and extracts it into `/var/lib/exa/bucketfs/<service>/<bucket>/<slc-name>/` on the VM.

The shared-directory mechanism serves a deployment that publishes no SSH port and no node key. Such a deployment shares a host directory into its VM instead. The deployment's own BucketFS mapping file, under `local/runtime/vm-shared/exa/`, names the VM-side path that serves each `<service>`/`<bucket>` pair. Joining that VM-side path to the shared host directory yields the host directory that serves the bucket. Extracting the tarball into `<slc-name>/` under it leaves the same tree at the same `/buckets/<service>/<bucket>/<slc-name>/` the SSH mechanism produces. The install opens no SSH session and runs no other tool.

The mechanism is selected from the literal inputs each one consumes, never from a version string. The SSH inputs take priority when both are present.

After placement the two mechanisms are identical. Both wait for the engine to reconcile the bucket, then issue the install script's own `ALTER SYSTEM SET SCRIPT_LANGUAGES` over the resolved SQL endpoint, preserving every pre-existing entry. Both therefore resolve the same four connection fields, and every BucketFS placement option applies to both.

The SQL endpoint the local install registers over is a launcher-managed forwarder whose port is assigned per deployment (`exasol config set --ports db:<port>`) and recorded in the same descriptor as `connection.dbPort`. `8563` is one deployment's assignment rather than a property of local Personal, so a host running several local deployments serves each on its own port. Local resolution therefore differs from cloud in one field only: its host default is `127.0.0.1`, the address the launcher forwards to. A local descriptor carrying no `connection.dbPort` is malformed, because the launcher always records the assigned port. No BucketFS password is needed on either mechanism, because neither uses the HTTP endpoint.
<!-- /DELTA:CHANGED -->

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Local install resolves the DB password from the deployment directory

* *GIVEN* a local Personal deployment whose `secrets.json` carries `.dbPassword`
* *WHEN* the local install resolves connection details without `--password`
* *THEN* the DB password MUST come from `secrets.json` `.dbPassword`
* *AND* a `--password` given on the command line MUST override it
* *AND* when neither resolves a password, the resolution MUST fail with a clear error
<!-- /DELTA:CHANGED -->

### Scenario: Local connection details resolve from the deployment directory

* *GIVEN* a local Personal deployment whose `deployment.json` carries `connection.dbPort`
* *WHEN* the local install resolves connection details with no overriding command-line flags
* *THEN* the DB host MUST come from `connection.host`, defaulting to `127.0.0.1` when that field is absent, because the launcher forwards the deployment's SQL endpoint to the invoking host
* *AND* the DB port MUST come from `connection.dbPort`, defaulting to `8563` when that field is absent, so on a host running several local deployments the install targets the database named by `--deployment` rather than whichever database answers `8563`
* *AND* the DB user MUST come from `connection.username`, defaulting to `sys` when that field is absent

<!-- DELTA:CHANGED -->
### Scenario: Command-line flags override descriptor-derived local values

* *GIVEN* a local Personal deployment
* *WHEN* the local install resolves connection details and any of `--host`, `--port`, `--user`, or `--password` is given on the command line
* *THEN* each provided flag MUST override the corresponding descriptor-derived value, under the same precedence the cloud path applies
* *AND* any of those values not given on the command line MUST fall back to the descriptor value
<!-- /DELTA:CHANGED -->

### Scenario: A local descriptor that omits the SQL port is reported

* *GIVEN* a local Personal deployment whose `deployment.json` carries no `connection.dbPort`, which the launcher always records
* *WHEN* the local install resolves connection details without `--port`
* *THEN* it MUST warn that the descriptor names no SQL port and that registering over the fallback `8563` risks hitting another local deployment
* *AND* an unreadable `deployment.json` MUST fail with a clear error rather than fall back to a built-in host or port

<!-- DELTA:NEW -->
### Scenario: The deployment directory selects the local install mechanism

* *GIVEN* a local Personal deployment
* *WHEN* the local install chooses its placement mechanism
* *THEN* it MUST choose the SSH mechanism when `deployment.json` carries a `.connection.sshPort` that reads as a port number and `local/node_access.pem` is readable
* *AND* when either SSH input is absent, it MUST choose the shared-directory mechanism if the deployment's BucketFS mapping resolves an existing host directory for the requested service and bucket
* *AND* when the deployment publishes no BucketFS mapping at all, it MUST fail with one error naming both mechanisms' prerequisites, because neither mechanism is available
* *AND* when the deployment publishes a BucketFS mapping that names no line for the requested service and bucket pair, it MUST fail naming that pair and the pairs the mapping does serve, because the mechanism is available and the requested pair is wrong
* *AND* when the mapping names the requested pair but the host directory it resolves is unusable, it MUST fail naming that directory rather than the requested pair, because the flags are correct
* *AND* it MUST reach this choice without reading or comparing a Personal version string

### Scenario: The shared-directory mechanism extracts into the deployment's own BucketFS directory

* *GIVEN* a local Personal deployment that selects the shared-directory mechanism, and a built SLC tarball on the invoking host
* *WHEN* the local install places the container
* *THEN* it MUST read the deployment's own BucketFS mapping to find the host directory that serves the requested service and bucket, rather than assume a fixed layout
* *AND* it MUST replace `<slc-name>/` under that host directory with the tarball's contents, so the install leaves exactly one tree and no file from an earlier install
* *AND* it MUST leave every file under that host directory outside `<slc-name>/` untouched, because the same bucket carries the operator's own UDF artifacts
* *AND* it MUST confirm that `exaudf/exaudfclient` landed executable before it registers anything
* *AND* it MUST open no SSH session and write nothing outside that host directory

### Scenario: The shared-directory destination is checked before anything is removed

* *GIVEN* a local Personal deployment on the shared-directory mechanism
* *WHEN* the local install resolves the destination for the given `--bfs-service`, `--bucket` and `--slc-name`
* *THEN* it MUST fail when the deployment's BucketFS mapping names no entry for that service and bucket pair, because extraction there would create a directory the engine never reconciles
* *AND* it MUST fail when the host directory that mapping resolves to does not already exist, because the deployment creates that directory and the install only adds the SLC under it
* *AND* it MUST require each of `--bfs-service`, `--bucket` and `--slc-name` to be a single path segment, rejecting a value that is empty, contains `/`, equals `.` or `..`, or starts with `-`
* *AND* it MUST fail when the destination it removes is not a direct child of the host directory the mapping resolved, because a name component that walks upwards would remove the deployment's own data
* *AND* it MUST fail when that host directory itself resolves outside the deployment directory
* *AND* it MUST print the resolved destination before it removes anything

### Scenario: Both local mechanisms register through the install script

* *GIVEN* a local Personal deployment on either mechanism
* *WHEN* the local install runs
* *THEN* it MUST resolve the DB host, port, user and password exactly as this feature's resolution scenarios specify, whichever mechanism places the container
* *AND* it MUST issue its own `ALTER SYSTEM SET SCRIPT_LANGUAGES` naming `/buckets/<service>/<bucket>/<slc-name>/`, as `container/personal-install` specifies
* *AND* `--bucket`, `--bfs-service`, `--slc-name`, `--host`, `--port`, `--user` and `--password` MUST apply on both mechanisms
* *AND* `--ssh-user` MUST apply on the SSH mechanism only, and the help text MUST say so

### Scenario: Re-running the local install replaces the installed SLC

* *GIVEN* a local Personal deployment that already carries a Rust SLC under the bucket at `<slc-name>`
* *WHEN* the local install runs again with a rebuilt tarball
* *THEN* the bucket MUST end with exactly one `<slc-name>` tree, holding the rebuilt tarball's contents
* *AND* the database MUST end with exactly one `RUST` entry in `SCRIPT_LANGUAGES`, replacing any earlier one
* *AND* every other language the deployment already had MUST remain registered
<!-- /DELTA:NEW -->
