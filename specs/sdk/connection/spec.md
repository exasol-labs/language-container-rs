# Feature: connection

Defines the SDK surface of a `CONNECTION` object lookup: the `ConnectionObject` credential struct and the lookup's error variant. Both are independent of connect-back.

## Scenarios

### Scenario: ConnectionObject is a public SDK type outside connect-back

* *GIVEN* the `exasol-udf-sdk` crate
* *WHEN* the `connection` module is referenced
* *THEN* it MUST expose a public `ConnectionObject` struct with public `kind`, `address`, `user`, and `password` fields, available unconditionally (no feature gate)
* *AND* the crate root and the `connect_back` module MUST re-export it, so existing imports keep compiling
* *AND* the `ConnectionObject` type MUST NOT reference any transport-layer type (it MUST NOT re-export or alias exarrow-rs internals)

### Scenario: A failed CONNECTION lookup has its own error variant

* *GIVEN* the `exasol-udf-sdk` crate
* *WHEN* `UdfContext::connection` fails
* *THEN* it MUST return `UdfError::Connection(String)`, whose display text is the carried text with no prefix and never mentions connect-back
* *AND* the `test-support` `TestContext` MUST return `UdfError::Connection("connection <name> does not exist")` for a name it was not given, matching the database's text
