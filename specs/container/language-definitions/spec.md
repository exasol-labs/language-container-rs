# Feature: language-definitions

Defines the SLC's `build_info/language_definitions.json` contract against the Exasol database's `language_definitions` v2 schema, and the key-path check that keeps the committed source and the shipped tarball copy conformant to it.

## Background

The database reads `build_info/language_definitions.json` out of the extracted tarball and schema-validates it during Engine/Nano initialization, when the tarball is installed as a custom SLC. A shape mismatch — a wrong root key, a re-typed field, a missing required key — fails validation with a `Mismatching schema path` error and restart-loops initialization, so the database never starts and the language container never runs. This contract governs the document's shape independently of `container/slim-image`, which covers only the build mechanics that stage and package the tarball, because the packaging pipeline can copy the file correctly while the file itself states the wrong contract.

The contract is checked by key path with `jq`, not by substring matching, because a renamed root key or a re-typed field can pass a substring check unnoticed — that gap is how an earlier version of this file shipped the wrong schema. The check runs against the committed source with no Docker build required, and separately against the tarball's shipped copy, so a wrong shape fails before the build as well as after it.

## Scenarios

### Scenario: Language definitions file is present and well-formed

* *GIVEN* the SLC tarball, whose `build_info/language_definitions.json` the database parses and schema-validates during Engine/Nano initialization when the tarball is installed as a custom SLC
* *WHEN* `build_info/language_definitions.json` is read from it
* *THEN* it MUST declare `schema_version` `2` and MUST carry its definitions under the root key `language_definitions`, and the root key `languages` MUST NOT be present, because the parser's root-level `required` check otherwise rejects the whole file and aborts initialization into a restart loop
* *AND* it MUST hold exactly one language definition, whose `aliases` is `["RUST"]`, whose `protocol` is `localzmq+protobuf`, whose `udf_client_path.executable` is `/exaudf/exaudfclient`, whose `parameters` is an array holding the single key/value object `{"key": "lang", "value": "rust"}`, and which carries a `deprecation` key present with the value `null`
* *AND* the definition MUST NOT carry an `arguments` key, so the `lang=rust` launch parameter has exactly one representation in the file and cannot drift against `parameters`
* *AND* the definition MAY retain `language_identifier` `rust`, which this parser does not consume

### Scenario: Language definition contract is asserted by key path on both copies

* *GIVEN* the repository's committed `build_info/language_definitions.json` and the copy the SLC tarball ships
* *WHEN* the language-definition contract is checked
* *THEN* each required field's absence, removal, or re-typing MUST fail the check, so a renamed root key or a re-typed field cannot pass unnoticed
* *AND* the copy the tarball ships MUST be byte-identical to the committed `build_info/language_definitions.json`, so the document that was checked is the document the database parses
* *AND* the contract MUST be checkable against the committed file alone, without building the container image, so a wrong shape fails before the build rather than after it
