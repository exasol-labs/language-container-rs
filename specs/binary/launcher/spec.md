# Feature: launcher

Provides the `exaudfclient` binary that the database invokes per UDF call, replicating the C++ launcher's argument contract, environment setup, and error-prefix conventions, then delegating to the host runtime.

## Background

The binary ships at `/exaudf/exaudfclient` and is invoked as `exaudfclient <ipc_socket_path> lang=rust [scriptOptionsParserVersion=1|2]`. It logs to stderr only (Exasol captures stderr as the UDF log), sets `HOME=/tmp` to match the C++ launcher, and surfaces fatal errors with the `F-UDF-CL-RUST-####` prefix. The `SCRIPT_OPTIONS_PARSER_VERSION` environment variable takes priority over the CLI argument, matching C++ behavior.

## Scenarios

### Scenario: Valid invocation delegates to the runtime

* *GIVEN* the binary invoked as `exaudfclient <socket> lang=rust`
* *WHEN* `main` parses the arguments
* *THEN* it MUST initialize stderr tracing, set `HOME=/tmp`, and construct the host runtime with the socket path and resolved parser version
* *AND* on a clean runtime exit it MUST return a success exit code

### Scenario: Wrong argument count is rejected

* *GIVEN* the binary invoked with fewer than two or more than three positional arguments
* *WHEN* `main` validates argument count
* *THEN* it MUST print a usage message to stderr
* *AND* it MUST return a non-zero exit code without constructing the runtime

### Scenario: Unsupported language is rejected with a prefixed error

* *GIVEN* the binary invoked with a second argument other than `lang=rust`
* *WHEN* `main` validates the language argument
* *THEN* it MUST print an `F-UDF-CL-RUST-` prefixed error to stderr
* *AND* it MUST return a non-zero exit code

### Scenario: Parser version env var overrides the CLI argument

* *GIVEN* an invocation that passes `scriptOptionsParserVersion=1` while `SCRIPT_OPTIONS_PARSER_VERSION=2` is set in the environment
* *WHEN* `main` resolves the parser version
* *THEN* it MUST use the environment value `2`
* *AND* the CLI argument MUST be used only when the environment variable is absent

### Scenario: Runtime failure surfaces a prefixed error

* *GIVEN* a valid invocation where the runtime returns an error
* *WHEN* `main` handles the runtime result
* *THEN* it MUST print the error to stderr with the `F-UDF-CL-RUST-` prefix
* *AND* it MUST return a failure exit code
