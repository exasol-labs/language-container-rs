# Feature: test-udfs-import-export

Provides the example UDF crates that demonstrate the IMPORT/EXPORT spec-generation hooks and the `rows_in_group` context accessor, and serve as fixtures for the integration tests. Core example fixtures are in `examples/test-udfs`; timestamp fixtures are in `examples/test-udfs-timestamps`; connect-back fixtures are in `examples/test-udfs-connect-back`.

## Background

Each example is a standalone cdylib crate depending only on `exasol-udf-sdk` and builds as a glibc-dynamic cdylib via a plain host `cargo build --release`. Every fixture that an integration scenario `dlopen`s MUST be wired into the CI "Build UDF .so artifacts (release)" `-p` allowlist. A fixture's unit tests build their `UdfContext` double from the `exasol-udf-sdk` `test-support` feature rather than from a hand-written `impl UdfContext`, so a change to the trait's required method set costs one edit in the SDK instead of one edit per fixture.

## Scenarios

### Scenario: import-export-spec generates IMPORT and EXPORT SQL from the spec payload

* *GIVEN* the `import-export-spec` crate exporting `IMPORT_SPEC_GEN`, `IMPORT_WORKER`, `EXPORT_SPEC_GEN`, and `EXPORT_WORKER` from one cdylib
* *WHEN* the `import_spec` hook of `IMPORT_SPEC_GEN` and the `export_spec` hook of `EXPORT_SPEC_GEN` parse their `json_spec`
* *THEN* the crate MUST enable both the SDK's `import` and `export` features and read each `json_spec` through them rather than through a hand-rolled parser, and each hook MUST return a `SELECT` that names its worker script qualified by `ctx.script_schema()` and passes a summary of the connection name, the parameters, and the column names it read
* *AND* `IMPORT_WORKER` MUST emit that summary together with the input schema it observed, so the summary reaches the client as imported rows
* *AND* `EXPORT_WORKER` MUST return its summary as a `UdfError`, because an `EXPORT` statement discards the generated `SELECT`'s result and the error channel is the only path back to the client
* *AND* the crate MUST appear in the workspace `members` and `default-members` lists and in the CI "Build UDF .so artifacts (release)" `-p` allowlist, because an integration scenario `dlopen`s it

### Scenario: import-export-spec's worker reads a variadic input schema at runtime

* *GIVEN* `IMPORT_WORKER` registered with a variadic `(...)` input list rather than a declared column list
* *WHEN* the generated `SELECT` calls it with the arguments the spec-generation hook chose
* *THEN* the crate MUST carry no `input(...)` annotation, so the runtime skips annotated-schema validation for it
* *AND* its `run` MUST build the emitted row from `ctx.input_column_count()` and `ctx.input_column(idx)`, reporting the column count and each column's declared name and type

### Scenario: rows-in-group reports the group row count the database sent

* *GIVEN* the `rows-in-group` crate with a `#[exasol_udf]` SET function returning `Result<(), UdfError>`
* *WHEN* its `run` reads `ctx.rows_in_group()` once and then iterates the group with `ctx.next()`
* *THEN* the crate MUST emit one row per group carrying the reported count and the number of rows it iterated
* *AND* it MUST read the count before the first `next()`, so the fixture proves the value is available up front rather than only after iteration
* *AND* the crate MUST appear in the workspace `members` and `default-members` lists and in the CI "Build UDF .so artifacts (release)" `-p` allowlist, because an integration scenario `dlopen`s it
