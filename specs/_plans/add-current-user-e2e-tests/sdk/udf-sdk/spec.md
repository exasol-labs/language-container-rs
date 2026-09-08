# Feature: udf-sdk

Defines the author-facing SDK — `UdfContext` and `UdfRun` traits, the `Value`/`ExaType` model, and the row-based emit surface — that UDF crates depend on without linking the host runtime or exarrow-rs. The connect-back surface is specified separately in `sdk/connect-back`; the ABI vtable and `emit-arrow` feature boundary are specified in `sdk/udf-abi`.

## Background

<!-- DELTA:CHANGED -->
The SDK crate is a pure contract crate: it defines the ABI, trait interfaces, and value types. It does not link the host runtime or exarrow-rs. The `#[exasol_udf]` proc-macro generates the cdylib entry point and vtable from a struct that implements `UdfRun`. Output is produced two ways selected by the UDF function's return type: an EMITS function returns `Result<(), UdfError>` and pushes rows through `ctx.emit()`; a RETURNS function returns `Result<Option<T>, UdfError>` and its value becomes the single output row.

`UdfContext` exposes plain handshake metadata to UDF code. Beyond the typed column accessors it provides `memory_limit()` and the `exascript_info` identity/origin accessors (`session_id`, `statement_id`, `node_id`, `node_count`, `vm_id`, `database_name`, `database_version`, `script_name`, `script_schema`, `current_user`, `current_schema`, `scope_user`), each sourced from `UdfMeta`; these are defaulted accessors (not feature-gated) so existing implementations keep compiling, overridden by the host context bridge to return the live value.

A UDF author needs a `UdfContext` to unit-test a UDF function without a live host. The SDK therefore ships that double itself, behind the non-default `test-support` cargo feature, so no author and no in-repo fixture hand-writes an `impl UdfContext`. The feature adds items only. It declares no dependency and enables no other feature, so a dependent crate's featureless test configuration keeps its meaning.
<!-- /DELTA:CHANGED -->

## Scenarios

<!-- DELTA:NEW -->
### Scenario: The test-support feature ships a reusable UdfContext test double

* *GIVEN* the `exasol-udf-sdk` crate built with the non-default `test-support` cargo feature
* *WHEN* a UDF author unit-tests a `#[exasol_udf]` function without a live host
* *THEN* the crate MUST expose a `test_support` module providing a `TestContext` that implements `UdfContext` over caller-supplied rows, so the author writes no `impl UdfContext`
* *AND* `TestContext` MUST offer a scalar constructor taking one `Vec<Value>` row whose `next` reports exhaustion, and a set constructor taking `Vec<Vec<Value>>` whose `next` advances a cursor across the group, with `num_columns` derived from the supplied row data rather than from a caller-set constant
* *AND* `TestContext` MUST record emitted rows and the `set_return` value as `emitted()` and `captured_return()`, where `captured_return()` distinguishes "never called" from "called with `None`", and MUST let the caller replace the default `emit` and `next` behavior with a caller-supplied `UdfError`, so a fixture can assert the runtime's ban on `emit` in RETURNS output and on `next` in scalar input
* *AND* `TestContext` MUST let the caller set each handshake metadata accessor and `debug_level`, every default matching the value the trait's own default returns, and MUST return `Err(UdfError::Type)` rather than panic from a `get` outside the current row or before the first `next` in set mode
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: The test-support feature ships a defaults-preserving UdfContext double

* *GIVEN* a test that asserts the default body of a provided `UdfContext` method, such as `memory_limit` returning `0` or `set_return` returning `UdfError::Unimplemented`
* *WHEN* that test needs a `UdfContext` instance
* *THEN* the `test_support` module MUST expose a `DefaultsCtx` that implements only the four required methods (`num_columns`, `get`, `emit`, `next`) and overrides no provided method, so the assertions observe the trait's own defaults instead of the double's re-implementation
* *AND* `DefaultsCtx` MUST report zero columns, return `Err(UdfError::Type)` from `get`, accept `emit` as a no-op, and report exhaustion from `next`
* *AND* a test that asserts a trait default MUST use `DefaultsCtx` rather than `TestContext`, because `TestContext` overrides those methods and would shadow the defaults under test
* *AND* the `test_support` module MUST NOT be compiled in a build that neither enables the `test-support` feature nor compiles the SDK crate's own unit tests, and the feature MUST NOT add, remove, or reorder any `UdfContext` method, so the `dyn UdfContext` vtable layout stays feature-independent as `sdk/udf-abi` requires
<!-- /DELTA:NEW -->
