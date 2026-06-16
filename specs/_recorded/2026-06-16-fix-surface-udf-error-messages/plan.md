# Plan: fix-surface-udf-error-messages

## Summary

Thread the text of a `UdfError` returned from a UDF `run` function through to the SQL error response, so the DB user sees the actual error message instead of a generic "UDF run returned error code 1". The error is currently dropped in the macro run shim because the shim has no channel to hand it back to the host. The fix adds a dedicated out-pointer parameter to the vtable `run` slot; the shim writes the error text into it and the host reads it after a non-zero return. UDF authors change no code — only a recompile against the new SDK is required.

## Design

### Context

When a UDF `run` returns `Err(UdfError)`, the generated run shim (in `exasol-udf-macros`) maps it to exit code `1` and discards the `UdfError` value. The host (`exa-udf-runtime::dispatch::run_batch`) sees only the non-zero code; it calls `bridge.take_last_error()`, but that slot is only ever written by connect-back failures, so for the common error-return path it is `None` and the host builds `"UDF run returned error code 1"` with no detail. The error text never reaches the DB error-close message, so users cannot diagnose UDF failures.

The shim holds only a thin `*mut c_void` (the double-indirected `&mut dyn UdfContext`) across the ABI boundary. To carry text back without adding any trait method or touching the connect-back `last_error` plumbing, the vtable `run` slot gains a second parameter: an out-pointer the shim fills with a heap-allocated C string. The host owns and frees that allocation after reading it.

- **Goals** — Surface the UDF-supplied error text in the SQL error; require no source changes in existing UDFs (recompile only); leave the `UdfContext` trait and the bridges' `last_error`/`take_last_error`/`record_error` connect-back plumbing untouched.
- **Non-Goals** — Adding methods to the `UdfContext` trait; changing the `F-UDF-CL-RUST-` prefix or error-code semantics; structured/typed error payloads over the wire; altering the panic path (code `2`).

### Decision

Change the `ExaUdfVTable.run` function-pointer signature from `unsafe extern "C" fn(ctx: *mut c_void) -> i32` to `unsafe extern "C" fn(ctx: *mut c_void, error_out: *mut *mut c_char) -> i32`. The macro shim writes the error display string to `*error_out` on the `Err` arm before returning code `1`; the host passes `&mut error_ptr` and reads the message after a non-zero return, then frees it with `CString::from_raw`. The vtable layout changes, so `EXA_UDF_ABI_VERSION` is bumped `3 → 4`; the loader's existing version check rejects stale v3 `.so` files. No `UdfContext` trait method is added and no bridge code changes.

#### Architecture

```
UDF run() ── Err(e) ──▶ run shim (macro)
                          │  if !error_out.is_null():
                          │    *error_out = CString::new(e.to_string()).into_raw()
                          │  return 1
                          ▼
        dispatch.rs reads error_ptr after non-zero return
        (no take_last_error needed for this path)
                          │  CString::from_raw(error_ptr).into_string()
                          ▼
        RuntimeError::Udf("... error code 1: <text>") ──▶ error-close ──▶ DB
```

#### Patterns

| Pattern | Where | Why |
|---------|-------|-----|
| Caller-freed out-pointer C string | `ExaUdfVTable.run` error_out arg | Mirrors the existing single-call hooks (`default_output_columns`, etc.) that already write `*result` C strings the host frees — one consistent ownership convention |
| ABI version gate | `EXA_UDF_ABI_VERSION` 3 → 4 | The loader already rejects mismatched versions, turning a layout change into a clear load-time error instead of UB |

### Consequences

| Decision | Alternatives Considered | Rationale |
|----------|------------------------|-----------|
| Add `error_out: *mut *mut c_char` to the vtable `run` slot and bump ABI to 4 | Add `record_error(&self, &str)` default method to the `UdfContext` trait | The out-pointer keeps the error channel inside the ABI the host already owns, needs zero UDF source changes (recompile only), and never widens the public `UdfContext` trait surface |
| Leave the bridges' `last_error`/`take_last_error`/`record_error` inherent methods unchanged | Consolidate them onto a trait method | Those methods are still the connect-back error sink; the UDF-error path is now independent, so no consolidation or dead code arises |
| New E2E scenario asserts the message text, leaving the prefix scenario unchanged | Modify the existing prefix scenario | The prefix scenario documents a distinct guarantee; a separate scenario isolates the regression and documents the bug before the fix |

## Features

| Feature | Status | Spec |
|---------|--------|------|
| sdk/udf-sdk | CHANGED | `specs/_plans/fix-surface-udf-error-messages/sdk/udf-sdk/spec.md` |
| runtime/host-dispatch | CHANGED | `specs/_plans/fix-surface-udf-error-messages/runtime/host-dispatch/spec.md` |
| integration/db-roundtrip | CHANGED | `specs/_plans/fix-surface-udf-error-messages/integration/db-roundtrip/spec.md` |

## Implementation Tasks

1. In `crates/exasol-udf-sdk/src/abi.rs`: change the `run` field type from `unsafe extern "C" fn(ctx: *mut std::ffi::c_void) -> i32` to `unsafe extern "C" fn(ctx: *mut std::ffi::c_void, error_out: *mut *mut c_char) -> i32`. Bump `EXA_UDF_ABI_VERSION` from `3` to `4`. Update the `run` doc comment to document the `error_out` ownership contract (host-allocated null on entry, shim may write a heap C string the host frees). Update the in-file test constants (`abi_version_and_vtable_layout`, `connect_back_feature_compiles`) from `3` to `4`, and update the `run_stub` definitions in the layout tests to take the second `error_out` parameter.
2. In `crates/exasol-udf-macros/src/lib.rs` `__exa_run_shim`: add `error_out: *mut *mut ::std::ffi::c_char` as the second parameter. In the `Err(e)` match arm, before yielding `1`, bind the error and write the text when `error_out` is non-null: `::std::result::Result::Ok(::std::result::Result::Err(e)) => { if !error_out.is_null() { let s = ::std::ffi::CString::new(::std::string::ToString::to_string(&e)).unwrap_or_default(); unsafe { *error_out = s.into_raw(); } } 1 }`. Keep the `Ok(())` arm (`0`) and the panic arm (`Err(_) => 2`) unchanged. The vtable `run: __exa_run_shim` reference is unchanged. [expert]
3. In `crates/exa-udf-runtime/src/dispatch.rs` `run_batch`: declare `let mut error_ptr: *mut std::ffi::c_char = std::ptr::null_mut();` before the call, pass `&mut error_ptr as *mut *mut std::ffi::c_char` as the second argument to `udf.run(ctx_ptr, ...)`. After a non-zero `rc`, recover the text from the out-pointer instead of `take_last_error`: `let extra = if !error_ptr.is_null() { Some(unsafe { std::ffi::CString::from_raw(error_ptr) }.into_string().unwrap_or_default()) } else { None };`, then keep the existing `match extra { Some(e) => ... , None => ... }` message construction and `RuntimeError::Udf` return. Do not remove the bridge or its `take_last_error` method (still used by connect-back). [expert]
4. Add a new integration scenario `udf_error_message_reaches_db` in `crates/it/tests/db_roundtrip.rs`, invoked right after `udf_error_surfaces_prefix`, that runs `SELECT json_field('not valid json')` and asserts the SQL error message contains the distinctive UDF text (e.g. `JSON parse error`). Leave `udf_error_surfaces_prefix` unchanged.
5. Run the full quality gate (build, test, clippy, fmt) and verify the new E2E scenario passes after the fix. E2E verification MUST be run with `EXASOL_DB_SERIES=2025-1` (Exasol 2025.1.11); the `udf_error_message_reaches_db` test must pass and the SQL error must contain the UDF-supplied text.

## Parallelization

| Parallel Group | Tasks |
|----------------|-------|
| Group A | Task 1 (ABI signature + version bump), Task 4 (E2E test authoring) |
| Group B | Task 2 (macro shim), Task 3 (host dispatch) |

Sequential dependencies:
- Group A → Group B (Task 2 and Task 3 depend on the new `run` signature and ABI version from Task 1)
- Group B → Task 5 (verification runs after all code changes)

Note: Task 4 only authors the test; it is independent of the signature change and is executed in Task 5.

## Dead Code Removal

None. The bridges' `last_error` field and `record_error`/`take_last_error` inherent methods are unchanged and still serve the connect-back error path.

## Verification

### Scenario Coverage

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| sdk/udf-sdk / Run shim surfaces UDF error text via an out-pointer parameter | Integration | `crates/it/tests/db_roundtrip.rs` | `udf_error_message_reaches_db` (end-to-end proof the shim writes the out-pointer the host reads) |
| runtime/host-dispatch / Dispatch reads UDF error text from the run out-pointer | Integration | `crates/it/tests/db_roundtrip.rs` | `udf_error_message_reaches_db` |
| integration/db-roundtrip / UDF error message content is surfaced without truncation | Integration | `crates/it/tests/db_roundtrip.rs` | `udf_error_message_reaches_db` |

The sdk/udf-sdk and runtime/host-dispatch scenarios are proven end-to-end by the same DB roundtrip test: the distinctive text reaches the SQL error only if the run shim writes the out-pointer, dispatch reads and frees it, and folds it into `RuntimeError::Udf`. No isolated unit test is added because the behavior has no pure-computation surface independent of the ABI wiring and live DB error-close path. The ABI version bump is additionally guarded by the in-file `abi.rs` unit tests asserting `EXA_UDF_ABI_VERSION == 4` (Task 1).

### Manual Testing

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| integration/db-roundtrip | `EXASOL_DB_SERIES=2025-1 cargo test -p it --test db_roundtrip` (requires Docker + Exasol image per CLAUDE.md) | `udf_error_message_reaches_db` passes against Exasol 2025.1.11; SQL error contains `JSON parse error` |
| sdk/udf-sdk | Recompile a UDF that returns `Err(UdfError::User("distinctive text".into()))` against the new SDK, register it, and `SELECT` it | DB error message includes `distinctive text`, not only `error code 1` |
| runtime/host-dispatch | Load a v3-ABI `.so` built against the previous SDK | Loader rejects it with a clear ABI version-mismatch error (no UB) |

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Build | `cargo build --release` | Exit 0 |
| Test | `EXASOL_DB_SERIES=2025-1 cargo test -p it --test db_roundtrip -- udf_error_message_reaches_db` | 0 failures |
| Lint | `cargo clippy --all-targets --all-features -- -D warnings` | 0 warnings |
| Format | `cargo fmt --check` | No changes |
