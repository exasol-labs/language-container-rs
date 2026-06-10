[language-container-rs](../README.md)

# Documentation

| Guide | What it covers |
|-------|----------------|
| [Writing a Rust UDF](writing-a-udf.md) | Prerequisites, implement, Value types, set UDFs, connect-back, build & deploy, unit testing |
| [Write-back from a UDF](write-back-guide.md) | Connect-back as a regular login, Serializable-isolation rules, autocommit, BIGINT emit, `query()` vs `query_arrow()`, pitfalls |
| [The Exasol UDF protocol](protocol.md) | ZMQ REQ/REP control channel, message types, handshake→run→cleanup lifecycle, MT_IMPORT credentials |
| [Cargo ecosystem](cargo-ecosystem.md) | Workspace crates, feature flags, cargo-exaudf subcommands, integration tests |
