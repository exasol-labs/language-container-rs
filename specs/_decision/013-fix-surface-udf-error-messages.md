# Decisions: fix-surface-udf-error-messages

## ADR: Surface UDF errors via a vtable `run` out-pointer, not a trait method

**ID:** surface-udf-errors-vtable-run-out-pointer
**Plan:** `fix-surface-udf-error-messages`
**Status:** Accepted

### Context

The generated run shim maps `Err(UdfError)` to exit code `1` and must also carry the error text to the host, or the database sees only `"UDF run returned error code 1"`. The channel must not widen the public `UdfContext` trait.

### Decision

The `ExaUdfVTable.run` function pointer takes a second parameter `error_out: *mut *mut c_char`. On the `Err` arm the shim writes a heap-allocated C string with the error's display text to `*error_out` when it is non-null, and returns the non-zero code. The host passes `&mut error_ptr`, reads the text after a non-zero return, and frees it with `libc::free`, like all other vtable result strings. The `UdfContext` trait and the `last_error`/`take_last_error`/`record_error` plumbing are unaffected.

### Options Considered

| Option | Verdict |
|--------|---------|
| `error_out` out-pointer on the vtable `run` slot | ✓ Chosen |
| `record_error(&self, &str)` default method on `UdfContext` | ✗ Widens the public trait surface |
| Encode the text in the `i32` return code | ✗ A status code cannot carry text |
| Thread-local error store | ✗ An explicit host-owned out-pointer is clearer |

### Consequences

A `.so` built against the previous ABI version is rejected at load time with a version-mismatch error. UDF authors only recompile. The connect-back `last_error` channel stays the sink for connect-back failures only.
