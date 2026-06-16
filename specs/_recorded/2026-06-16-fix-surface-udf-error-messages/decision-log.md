# Decision Log: fix-surface-udf-error-messages

Date: 2026-06-16

## Interview

**Q:** What is the desired outcome?
**A:** Two things: (1) a new E2E integration test that documents the current broken
behavior by asserting the actual UDF error text appears in the SQL error (this test
FAILS today, proving the bug); (2) a minimal fix that threads the error message
through to the DB. Changes must be as concise as possible.

**Q:** Where is the bug?
**A:** The `UdfError` returned from a UDF `run` is dropped in the macro run shim
(`crates/exasol-udf-macros/src/lib.rs`), which maps `Err` to code `1` without
carrying the message anywhere. The host (`dispatch.rs`) then only finds `None` in
`take_last_error` (that slot is written only by connect-back failures), so the DB
sees a generic "UDF run returned error code 1".

**Q (refinement):** Should the fix add a `record_error` method to the `UdfContext`
trait, as the first draft of this plan proposed?
**A:** No. No new methods on the `UdfContext` trait. UDFs already return
`Err(UdfError)` — the text is there, it is just discarded. UDFs must not need to
change their code at all; a recompile is acceptable (an ABI change is fine).

**Q (refinement):** What mechanism instead?
**A:** Change the `run` field in `ExaUdfVTable` from
`unsafe extern "C" fn(ctx: *mut c_void) -> i32` to
`unsafe extern "C" fn(ctx: *mut c_void, error_out: *mut *mut c_char) -> i32`. The
macro-generated shim fills `*error_out` with a heap-allocated `CString` on `Err(e)`,
writing `e.to_string()`. The host passes `&mut error_ptr` as the second arg, reads
the message after a non-zero return, and frees it with `CString::from_raw`. This
requires an ABI version bump (3 → 4) and zero changes to `UdfContext` or any
bridge's `last_error`/`take_last_error` plumbing.

## Design Decisions

### [1] Surface UDF errors via a vtable `run` out-pointer, not a trait method

- **Decision:** Add a second parameter `error_out: *mut *mut c_char` to the
  `ExaUdfVTable.run` function pointer. The shim writes a heap-allocated, host-freed
  C string holding the error display text to `*error_out` on the `Err` arm; the host
  reads and frees it. Bump `EXA_UDF_ABI_VERSION` from 3 to 4. The `UdfContext` trait
  and all bridge plumbing are untouched.
- **Alternatives:** (a) Add a `record_error(&self, &str)` default method to the
  `UdfContext` trait and have the bridges override it — REJECTED by the user: no new
  trait methods, and it widens the public trait surface unnecessarily. (b) Encode the
  error string into the `i32` return code — rejected, the return is a status code and
  cannot carry text. (c) Store the error in a thread-local — rejected, an explicit
  out-pointer owned by the host is clearer and matches the existing single-call hook
  convention (`default_output_columns` etc. already write caller-freed `*result` C
  strings).
- **Rationale:** Keeps the error channel inside the ABI the host already owns,
  requires zero UDF source changes (recompile only), reuses the established
  caller-freed-C-string ownership convention, and leaves the connect-back
  `last_error` sink independent.
- **Promotes to ADR:** yes

### [2] Reject the `record_error` trait-method approach from the prior draft

- **Decision:** Do not promote `record_error` to a `UdfContext` trait method and do
  not consolidate the bridges' inherent `record_error(String)` onto a trait method.
  Leave `last_error` / `take_last_error` / `record_error` on the bridges exactly as
  they are; they remain the connect-back error sink only.
- **Alternatives:** The prior plan draft added the default trait method and rewrote
  the bridges — rejected per the user's explicit constraint that the `UdfContext`
  trait gains no methods and that the fix touch as little host plumbing as possible.
- **Rationale:** The out-pointer mechanism fully decouples the UDF-error path from
  connect-back, so no trait change and no bridge consolidation are needed; this
  avoids both a public-API change and accidental dead code.
- **Promotes to ADR:** no

### [3] New E2E scenario, prefix scenario left intact

- **Decision:** Add a new `db-roundtrip` scenario asserting the message text
  (`udf_error_message_reaches_db`) and leave the existing
  `UDF runtime error surfaces a prefixed message` scenario unchanged.
- **Alternatives:** Modify the existing prefix scenario to also check the text —
  rejected; the prefix guarantee is distinct and worth keeping isolated.
- **Rationale:** A separate scenario documents the bug before the fix and keeps the
  two guarantees (prefix present vs. text preserved) independently verifiable.
- **Promotes to ADR:** no

## Review Findings

<!-- Populated by speq-implement after code review. -->
