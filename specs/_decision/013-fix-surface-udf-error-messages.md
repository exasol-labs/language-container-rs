# Decisions: fix-surface-udf-error-messages

## ADR: Surface UDF errors via a vtable `run` out-pointer, not a trait method

**ID:** surface-udf-errors-vtable-run-out-pointer
**Plan:** `fix-surface-udf-error-messages`
**Status:** Accepted

### Context

When a UDF `run` returns `Err(UdfError)`, the generated run shim mapped the error to exit code `1` and discarded the `UdfError` value entirely. The host (`dispatch.rs`) then found `None` in `take_last_error` (written only by connect-back failures), so the DB saw only `"UDF run returned error code 1"` with no detail. An early plan draft proposed adding a `record_error(&self, &str)` default method to the `UdfContext` trait to carry the error text out, but this was rejected by the user as widening the public trait surface unnecessarily.

### Decision

Add a second parameter `error_out: *mut *mut c_char` to the `ExaUdfVTable.run` function pointer. The generated run shim writes a heap-allocated, host-freed C string holding the error's display text to `*error_out` on the `Err` arm (when `error_out` is non-null) and returns the non-zero error code. The host passes `&mut error_ptr`, reads the text after a non-zero return, and frees the allocation using the `malloc`/`libc::free` C-allocator convention, consistent with all other vtable result strings. `EXA_UDF_ABI_VERSION` is bumped from 3 to 4. The `UdfContext` trait and all bridge `last_error`/`take_last_error`/`record_error` plumbing are untouched.

### Options Considered

| Option | Verdict |
|--------|---------|
| Add `error_out: *mut *mut c_char` out-pointer to the vtable `run` slot; bump ABI to 4 | ✓ Chosen — keeps the error channel inside the ABI the host already owns; zero UDF source changes (recompile only); reuses the established caller-freed-C-string convention; leaves the `UdfContext` trait and connect-back `last_error` sink independent |
| Add `record_error(&self, &str)` default method to the `UdfContext` trait | ✗ Rejected — widens the public trait surface; requires all bridge implementations to be aware of the UDF-error path; rejected by the user |
| Encode the error string into the `i32` return code | ✗ Rejected — the return is a status code and cannot carry text |
| Store the error in a thread-local | ✗ Rejected — an explicit out-pointer owned by the host is clearer and matches the existing single-call hook convention |

### Consequences

The `ExaUdfVTable.run` slot has a two-parameter signature from ABI version 4 onwards. All `.so` files compiled against ABI v3 are rejected at load time with a clear version-mismatch error, not silent UB. UDF authors require only a recompile — no source changes. The connect-back `last_error` channel remains the exclusive error sink for connect-back failures; the UDF-error path is fully independent.
