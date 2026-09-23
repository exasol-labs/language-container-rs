# Feature: udf-spec-hooks

Defines the `UdfRun` single-call hook defaults, the `generate_sql_for_import_spec` / `generate_sql_for_export_spec` JSON payload contract, the `UdfContext` accessors for the current input group's row count and the database's declared iteration axes, the `import`/`export` typed-spec-parsing features, and the `test-support` feature's `UdfContext` test doubles. The core per-row `UdfContext`/`UdfRun` surface — column access, `Value`/`ExaType`, RETURNS/EMITS output — is specified in `sdk/udf-sdk`. The ABI vtable slot shape for these hooks is specified in `sdk/udf-spec-abi`.

## Background

`UdfRun` declares one hook per single-call function id (`default_output_columns`, `generate_sql_for_import_spec`, `generate_sql_for_export_spec`, `virtual_schema_adapter_call`), all defaulted to `UdfError::Unimplemented` so a struct providing only `run` still compiles. The two spec-generation hooks share `virtual_schema_adapter_call`'s `(ctx, json_spec) -> Result<String, UdfError>` shape. `UdfContext` additionally reports the current input group's row count (`rows_in_group`) and the database's declared input/output iteration axes (`input_type`, `output_type`), both provided (defaulted) accessors ungated by any cargo feature.

A UDF author needs a `UdfContext` to unit-test a UDF function without a live host. The SDK therefore ships that double itself, behind the non-default `test-support` cargo feature, so no author and no in-repo fixture hand-writes an `impl UdfContext`. The feature adds items only. It declares no dependency and enables no other feature, so a dependent crate's featureless test configuration keeps its meaning.

The SDK also parses the IMPORT and EXPORT specification payload for the authors who want it, behind two non-default cargo features named for the statement each serves. The `import` feature ships `ImportSpec` and the `export` feature ships `ExportSpec`, so an author who writes one kind of spec-generation UDF compiles only that kind's types. Each feature adds `serde` and `serde_json` as optional dependencies, so a default UDF build takes neither. The ABI hook signature carries the raw JSON string in every build configuration, and the typed structs are a parse step the author opts into.

## Scenarios

### Scenario: UdfRun default single-call hooks return Unimplemented

* *GIVEN* a struct that implements `UdfRun` providing only `run`
* *WHEN* a single-call hook (`default_output_columns`, `generate_sql_for_import_spec`, `generate_sql_for_export_spec`, `virtual_schema_adapter_call`) is invoked
* *THEN* the default implementation MUST return `UdfError::Unimplemented`
* *AND* the trait MUST declare `generate_sql_for_import_spec` and `generate_sql_for_export_spec` as `fn(ctx: &mut dyn UdfContext, json_spec: &str) -> Result<String, UdfError>`, the same shape `virtual_schema_adapter_call` already uses
* *AND* the trait MUST compile without the author providing those hooks

### Scenario: Spec-generation hooks receive the specification as a JSON mirror of the proto message

* *GIVEN* a `generate_sql_for_import_spec` or `generate_sql_for_export_spec` hook
* *WHEN* the host invokes it for an `IMPORT ... FROM SCRIPT` or `EXPORT ... INTO SCRIPT` statement
* *THEN* the `json_spec` argument MUST be a JSON object carrying every field of the matching proto message (`import_specification_rep`, `export_specification_rep`) under that field's proto name
* *AND* a `repeated` field MUST appear as a JSON array, a nested message as a JSON object, and an absent `optional` field as JSON `null`, so the object's key set never varies with what the database populated
* *AND* `parameters` MUST appear as an array of `{"key":…,"value":…}` objects, mirroring `key_value_pair` rather than collapsing to a map
* *AND* a `column_type` enum value MUST appear as its proto variant name, for example `"PB_DOUBLE"`

### Scenario: UdfContext reports the row count of the current input group

* *GIVEN* the `UdfContext` trait
* *WHEN* a UDF queries how many input rows the database placed in the group it is reading
* *THEN* the trait MUST provide `rows_in_group(&self) -> u64` returning the count the database reported for the current input batch
* *AND* it MUST be a provided (defaulted) trait method returning `0`, the engine's own value for a call with no group defined, so existing implementations keep compiling
* *AND* for SCALAR input it MUST return the current vector-chunk size the engine reported rather than a group size, so a SCALAR UDF MUST NOT size a whole-input buffer from it
* *AND* the value MUST stay constant across every `next()` step of one group and across the read that follows `next()` reporting exhaustion, because the accessor reports the last batch the database sent, so a UDF MAY read it at any point in the group
* *AND* it MUST NOT be gated behind the `connect-back` feature, because the count is plain DB-supplied context

### Scenario: UdfContext reports the declared input and output iteration axes

* *GIVEN* the `UdfContext` trait
* *WHEN* a UDF asks how the database declared the script it runs in
* *THEN* the trait MUST provide `input_type(&self) -> Option<InputType>` and `output_type(&self) -> Option<OutputType>`, where `InputType` is `Scalar` or `Set` and `OutputType` is `Returns` or `Emits`
* *AND* both MUST be provided (defaulted) trait methods returning `None`, which reports a context carrying no host metadata, so existing implementations keep compiling
* *AND* the two enums MUST name the shapes an author writes in `CREATE SCRIPT`, not the `PB_EXACTLY_ONCE` and `PB_MULTIPLE` proto variants the wire carries
* *AND* both MUST be declared unconditionally, behind no cargo feature, because the axes are plain DB-supplied handshake metadata

### Scenario: The import and export features parse the specification payload into typed structs

* *GIVEN* the `exasol-udf-sdk` crate built with the non-default `import` feature, the non-default `export` feature, or both
* *WHEN* a spec-generation hook calls `ImportSpec::from_json(json_spec)` or `ExportSpec::from_json(json_spec)`
* *THEN* `import` MUST ship `ImportSpec` and `export` MUST ship `ExportSpec`, each mirroring the `json_spec` shape the spec-generation scenario pins field for field under the proto field names, without normalizing any value, so this crate adds no second owner of the proto-to-`ExaType` mapping that `exa-zmq-protocol` holds
* *AND* each parser MUST return `Err(UdfError::Type)` carrying the parse error for a payload it cannot read, and MUST ignore an unknown JSON field rather than reject it, so a `.so` built against this SDK keeps parsing a payload that a later proto field widened
* *AND* either feature alone MUST compile and MUST ship only its own specification type, so an author who writes one kind of spec-generation UDF carries no code for the other
* *AND* `connection_information` MUST parse into the SDK's own `ConnectionObject` and `parameters` into an ordered list of key-value pairs, each defined once for the two features rather than once per feature, so no new type names a concept the SDK already names

### Scenario: The test-support feature ships a reusable UdfContext test double

* *GIVEN* the `exasol-udf-sdk` crate built with the non-default `test-support` cargo feature
* *WHEN* a UDF author unit-tests a `#[exasol_udf]` function without a live host
* *THEN* the crate MUST expose a `test_support` module providing a `TestContext` that implements `UdfContext` over caller-supplied rows, so the author writes no `impl UdfContext`
* *AND* `TestContext` MUST offer a scalar constructor taking one `Vec<Value>` row whose `next` reports exhaustion, and a set constructor taking `Vec<Vec<Value>>` whose `next` advances a cursor across the group, with `input_column_count` derived from the supplied row data rather than from a caller-set constant
* *AND* `TestContext` MUST record emitted rows and the `set_return` value as `emitted()` and `captured_return()`, where `captured_return()` distinguishes "never called" from "called with `None`", and MUST let the caller replace the default `emit` and `next` behavior with a caller-supplied `UdfError`, so a fixture can assert the runtime's ban on `emit` in RETURNS output and on `next` in scalar input
* *AND* `TestContext` MUST let the caller set each handshake metadata accessor, `rows_in_group`, `input_type`, `output_type`, and `debug_level`, every default matching the value the trait's own default returns, and supply the input and output `ColumnInfo` lists (unset by default) so a fixture can unit-test a UDF that reads its own schema, and MUST return `Err(UdfError::Type)` rather than panic from a `get` outside the current row or before the first `next` in set mode

### Scenario: The test-support feature ships a defaults-preserving UdfContext double

* *GIVEN* a test that asserts the default body of a provided `UdfContext` method, such as `memory_limit` returning `0` or `set_return` returning `UdfError::Unimplemented`
* *WHEN* that test needs a `UdfContext` instance
* *THEN* the `test_support` module MUST expose a `DefaultsCtx` that implements only the four required methods (`input_column_count`, `get`, `emit`, `next`) and overrides no provided method, so the assertions observe the trait's own defaults instead of the double's re-implementation
* *AND* `DefaultsCtx` MUST report zero columns, return `Err(UdfError::Type)` from `get`, accept `emit` as a no-op, and report exhaustion from `next`
* *AND* a test that asserts a trait default MUST use `DefaultsCtx` rather than `TestContext`, because `TestContext` overrides those methods and would shadow the defaults under test
* *AND* the `test_support` module MUST NOT be compiled in a build that neither enables the `test-support` feature nor compiles the SDK crate's own unit tests, and the feature MUST NOT add, remove, or reorder any `UdfContext` method, so the `dyn UdfContext` vtable layout stays feature-independent as `sdk/udf-abi` requires
