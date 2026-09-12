# Decisions: fix-launcher-start-artefact

## ADR: The launcher writes nothing and resolves no parser version

**ID:** launcher-start-artefact
**Plan:** fix-launcher-start-artefact
**Status:** Accepted

### Context

Every VM start wrote its argument list to `/tmp/exaudf_started.txt`, a fixed path inside the sandbox that concurrent VMs on a node share and overwrite. The file was a bring-up aid: it proved the binary had been executed when the handshake never happened. Nothing asserts on it — the integration harness only lists it, alongside other names, in the log dump it prints after a failed scenario.

The launcher also resolved a script-options parser version from `EXAUDF_PARSER_VERSION` or a `parser_version=N` argument and only logged the result. The reference C++ launcher uses that version to pick between two script-option dialects; this runtime has one, reading `%udf_object` and `%udf_debug_level` line by line.

### Decision

The launcher creates no files. The invocation arguments go to stderr at debug level, on the channel `%udf_debug_level` and the script-output redirect already carry. The parser-version argument stays in the invocation contract, accepted and ignored, so the DB may pass it.

### Options Considered

| Option | Verdict |
|--------|---------|
| Drop the write; log the arguments at debug level | ✓ Chosen — no syscall on the default path, one diagnostics channel |
| Gate the write behind an environment variable | ✗ Rejected — the DB execs the binary directly in a container with no shell, so nothing could set it |
| Gate the write behind the resolved debug level | ✗ Rejected — the level arrives with the handshake, after the moment the file existed to prove |
| Select a script-option dialect by parser version | ✗ Rejected — there is one dialect; a second exists only in the reference launcher |

### Consequences

A UDF that never reaches the handshake leaves no trace in the sandbox; the DB's own `VM crashed` report and stderr remain the evidence. `EXAUDF_PARSER_VERSION` and `parser_version=N` no longer have any effect, neither having had one beyond a log line.
