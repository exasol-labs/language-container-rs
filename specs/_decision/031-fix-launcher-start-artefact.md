# Decisions: fix-launcher-start-artefact

## ADR: The launcher writes nothing and resolves no parser version

**ID:** launcher-start-artefact
**Plan:** fix-launcher-start-artefact
**Status:** Accepted

### Context

A fixed-path start file in the sandbox is shared and overwritten by concurrent VMs, and nothing asserts on it. A parser version selects between two script-option dialects, and this runtime has one.

### Decision

The launcher creates no files. It writes the invocation arguments to stderr at debug level, the channel `%udf_debug_level` and the script-output redirect already use. The parser-version argument stays in the invocation contract, accepted and ignored.

### Options Considered

| Option | Verdict |
|--------|---------|
| Log the arguments at debug level, write no file | ✓ Chosen |
| Gate the write behind an environment variable | ✗ The DB execs the binary in a container with no shell, so nothing sets it |
| Gate the write behind the debug level | ✗ The level arrives with the handshake, after the moment the file would prove |
| Select a dialect by parser version | ✗ This runtime has one dialect |

### Consequences

A UDF that never reaches the handshake leaves no trace in the sandbox, and the DB's `VM crashed` report and stderr remain the evidence. `EXAUDF_PARSER_VERSION` and `parser_version=N` have no effect.
