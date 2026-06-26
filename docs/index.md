[language-container-rs](../README.md)

# Documentation

| Guide | What it covers |
|-------|----------------|
| [Mission](../specs/mission.md) | What lc-rs is, who it's for, core capabilities, tech stack, commands |
| [Architecture](../specs/architecture.md) | Layered pipeline + data flow diagram, design decisions, project structure, data-type mapping |
| [Installation](installation.md) | Build, upload and register the language container; read the BucketFS write password |
| [Writing a Rust UDF](writing-a-udf.md) | Scaffold, macro, UdfContext API, Value types, Decimal, ExaType, UdfError, scalar & SET UDFs, connect-back, build & deploy, unit testing |
| [The Exasol UDF protocol](protocol.md) | ZMQ REQ/REP control channel, message types, handshake→run→cleanup lifecycle, MT_IMPORT credentials |
| [Cargo ecosystem](cargo-ecosystem.md) | Workspace crates, feature flags, cargo-exasol-udf subcommands, integration tests |
| [Debugging Rust UDFs](debugging.md) | `SET SESSION SCRIPT OUTPUT ADDRESS`, `%udf_debug_level`, `udf_log!`, `ctx.debug_level()` |
