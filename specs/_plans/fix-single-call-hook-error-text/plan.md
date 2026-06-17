# Plan: fix-single-call-hook-error-text

## Summary

Fix GitHub issue #11 by making the three single-call hook helpers in `crates/exa-udf-runtime/src/loader.rs` read the hook's `UdfError` text from the result out-pointer before freeing it, so VS-adapter and other single-call errors surface their real message in SQL instead of the opaque `returned error code 1`.

## Context

Single-call hooks (`default_output_columns`, `generate_sql_for_import/export_spec`, `virtual_schema_adapter_call`) discard their own `UdfError` text when they return a non-zero code. The `#[exasol_udf]` macro shim writes the real error message into the result out-pointer, but `call_noarg_hook`, `call_arg_hook`, and `call_ctx_arg_hook` all `libc::free` that buffer on the `rc != 0` path and report only a generic `single-call hook <name> returned error code <rc>` string. Commit 8a5aa52 ("surface UDF run error text") wired the error out-pointer into the `run` slot in `dispatch.rs`, but the single-call helpers were not updated. The only surviving error channel for single-call hooks is `SingleCallContext::take_last_error`, which is populated solely by connect-back/connection/cluster-ip failures — not by a hook's returned `UdfError`.

This is a patch-level bug fix: no behavior changes beyond surfacing the already-written error text. Workspace version bumps `0.11.0` → `0.11.1`.

## Design

Skipped — this is a small, localized bug fix with no architectural change. The fix reuses the existing `crate::single_call::take_c_string` helper (which already handles null and frees via the C allocator) instead of the current free-then-discard pattern.

## Features

| Feature | Status | Spec |
|---------|--------|------|
| dispatch-single-call | CHANGED | delta: `runtime/dispatch-single-call/spec.md` → `specs/runtime/dispatch-single-call/spec.md` |

## Implementation Tasks

1. Create feature branch `feat/fix-single-call-hook-error-text` off `main`.
2. Fix the three helpers in `crates/exa-udf-runtime/src/loader.rs` (`call_noarg_hook`, `call_arg_hook`, `call_ctx_arg_hook`): on `rc != 0`, read the out-pointer via `crate::single_call::take_c_string(out)` and surface it; fall back to the generic `single-call hook {name} returned error code {rc}` message only when the read text is empty. The error path MUST NOT separately `libc::free` the buffer (take_c_string frees it).
3. Add a `#[cfg(test)]` module at the end of `crates/exa-udf-runtime/src/loader.rs` with unit tests that call the private helpers using inline `extern "C"` function pointers (no `.so` fixture). Cover: (a) hook returns rc != 0 with a non-empty message in the out-pointer → returned `RuntimeError::Udf` contains that message; (b) hook returns rc != 0 with a null/empty out-pointer → returned error uses the generic `returned error code` text; (c) hook returns rc == 0 → existing success path still returns the written string. Test buffers MUST be allocated with the C allocator (e.g. `libc::strdup`) so `take_c_string`'s `libc::free` is sound.
4. Applying the delta scenario from `runtime/dispatch-single-call/spec.md` into `specs/runtime/dispatch-single-call/spec.md` is handled by `speq record` later; for this plan only the delta file is authored.
5. Bump workspace `version = "0.11.0"` → `version = "0.11.1"` in `Cargo.toml`.
6. Run the verification checklist (build, test, clippy, fmt).
7. Open a PR against `main` with a `fix:` Conventional Commit title referencing issue #11.

## Parallelization

| Parallel Group | Tasks |
|----------------|-------|
| Group A | Task 2 (code fix + unit tests in same file, Task 3), Task 5 (version bump) |

Sequential dependencies:
- Task 1 (branch) → Group A → Task 6 (verify) → Task 7 (PR)
- Tasks 2 and 3 edit the same file (`loader.rs`) and must be done together by one worker, not split.

## Dead Code Removal

| Type | Location | Reason |
|------|----------|--------|
| Code (inline) | `crates/exa-udf-runtime/src/loader.rs` error-path `if !out.is_null() { libc::free(...) }` blocks (×3) | Replaced by `take_c_string`, which reads then frees; the standalone free becomes a use-after-free if kept |

## Verification

### Scenario Coverage

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| Single-call hook error text is surfaced when rc != 0 | Unit | `crates/exa-udf-runtime/src/loader.rs` (`#[cfg(test)] mod tests`) | `error_text_surfaced_when_rc_nonzero` |
| Single-call hook error text is surfaced when rc != 0 (empty/null fallback) | Unit | `crates/exa-udf-runtime/src/loader.rs` (`#[cfg(test)] mod tests`) | `generic_message_when_error_text_empty` |
| Single-call hook error text is surfaced when rc != 0 (success path unaffected) | Unit | `crates/exa-udf-runtime/src/loader.rs` (`#[cfg(test)] mod tests`) | `success_path_returns_written_string` |

Unit tests are justified here: the helpers are pure FFI plumbing over function pointers with no ZMQ/DB I/O, exercised directly with inline `extern "C"` hooks. This is isolated computation, the one case where unit tests are preferred over integration tests.

### Manual Testing

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| dispatch-single-call | `cargo +1.91 test -p exa-udf-runtime loader::tests` | 3 passing tests; the error-text test asserts the hook message (not `error code 1`) is in the `RuntimeError::Udf` payload |

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Build | `cargo build --release -p exa-udf-runtime` | Exit 0 |
| Test | `cargo test -p exa-udf-runtime` | 0 failures |
| Lint | `cargo clippy --all-targets --all-features -- -D warnings` | 0 errors/warnings |
| Format | `cargo fmt --check` | No changes |
