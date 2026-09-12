# Feature: launcher

Provides the `exaudfclient` binary that the database invokes per UDF call, replicating the C++ launcher's argument contract, environment setup, and error-prefix conventions, then delegating to the host runtime.

## Background

The binary ships at `/exaudf/exaudfclient` and is invoked as `exaudfclient <ipc_socket_path> lang=rust [scriptOptionsParserVersion=1|2]`. It logs to stderr only (Exasol captures stderr as the UDF log), sets `HOME=/tmp` to match the C++ launcher, and surfaces fatal errors with the `F-UDF-CL-RUST-####` prefix. The Rust SLC parses script options in one dialect, so the parser-version argument is accepted and ignored.

## Scenarios

### Scenario: Valid invocation delegates to the runtime

* *GIVEN* the binary invoked as `exaudfclient <socket> lang=rust`
* *WHEN* `main` parses the arguments
* *THEN* it MUST initialize stderr tracing, set `HOME=/tmp`, and construct the host runtime with the socket path
* *AND* on a clean runtime exit it MUST return a success exit code

### Scenario: Wrong argument count is rejected

* *GIVEN* the binary invoked with fewer than two positional arguments
* *WHEN* `main` validates argument count
* *THEN* it MUST print a usage message to stderr
* *AND* it MUST return a non-zero exit code without constructing the runtime

### Scenario: Unsupported language is rejected with a prefixed error

* *GIVEN* the binary invoked with a second argument other than `lang=rust`
* *WHEN* `main` validates the language argument
* *THEN* it MUST print an `F-UDF-CL-RUST-` prefixed error to stderr
* *AND* it MUST return a non-zero exit code

### Scenario: The script-options parser version is ignored

* *GIVEN* an invocation that passes `scriptOptionsParserVersion=2`
* *WHEN* `main` parses the arguments
* *THEN* it MUST behave as if the argument were absent

### Scenario: Starting leaves no artefact in the sandbox

* *GIVEN* any invocation
* *WHEN* `main` starts
* *THEN* it MUST NOT create or write any file
* *AND* it MUST report the invocation arguments on stderr at debug level

### Scenario: Runtime failure surfaces a prefixed error

* *GIVEN* a valid invocation where the runtime returns an error
* *WHEN* `main` handles the runtime result
* *THEN* it MUST print the error to stderr with the `F-UDF-CL-RUST-` prefix
* *AND* it MUST return a failure exit code
