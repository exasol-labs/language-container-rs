# Decision Log: fix-single-call-hook-error-text

Date: 2026-06-17

## Interview

No clarifying questions were needed. The root cause, fix location, and expected
behaviour are unambiguous from GitHub issue #11 and the code in
`crates/exa-udf-runtime/src/loader.rs`.

**Q:** (implicit) Where is the error text being lost?
**A:** All three single-call hook helpers (`call_noarg_hook`, `call_arg_hook`,
`call_ctx_arg_hook`) free the result out-pointer on `rc != 0` and report only a
generic `returned error code <rc>` string, discarding the `UdfError` text the
`#[exasol_udf]` macro shim wrote into the buffer.

**Q:** (implicit) What is the expected behaviour after the fix?
**A:** On a non-zero return code, the helper reads the out-pointer text and
surfaces it; the generic message is used only when that text is null/empty.

## Design Decisions

### [1] Reuse `take_c_string` instead of free-then-discard

- **Decision:** On the `rc != 0` path, call `crate::single_call::take_c_string(out)` to read and own the hook-written message, then surface it; fall back to the generic message only when empty.
- **Alternatives:** Mirror the `dispatch.rs` `run`-path pattern verbatim (separate `error_ptr`, manual null check, `format!("...: {e}")`). Rejected because the single-call helpers already have the message in their existing `out` pointer — `take_c_string` already handles the null case and the free, so it is strictly simpler and removes the now-redundant manual `libc::free`.
- **Rationale:** Minimal change, no new allocation/free paths, reuses the same C-allocator-crossing helper used by the success path, eliminates a use-after-free risk.
- **Promotes to ADR:** no

### [2] Unit tests in `loader.rs` with inline `extern "C"` hooks, not a new `.so` fixture

- **Decision:** Add a `#[cfg(test)]` module to `loader.rs` that calls the private helpers directly with inline `extern "C"` function pointers; no new cdylib fixture or ZMQ mock-DB integration test.
- **Alternatives:** Add error-returning hooks to `test-udfs/single-call-fixture` and exercise them through the existing ZMQ integration test in `crates/exa-udf-runtime/tests/single_call.rs`. Rejected as heavier: it requires building a new `.so`, wiring fixture hooks, and a full protocol roundtrip to test FFI plumbing that has no I/O.
- **Rationale:** The helpers are pure FFI plumbing over function pointers. Inline hooks exercise exactly the rc/out-pointer logic being fixed with no build or I/O dependency — the legitimate case for a unit test over an integration test per the project mission.
- **Promotes to ADR:** no

### [3] Patch version bump 0.11.0 → 0.11.1

- **Decision:** Bump workspace `version` to `0.11.1`.
- **Alternatives:** Minor bump (`0.12.0`). Rejected — no new feature or API surface, behaviour only becomes correct.
- **Rationale:** Bug fix only; SemVer patch level.
- **Promotes to ADR:** no

## Review Findings

<!-- Populated by speq-implement after code review. -->
