# Verification Report: fix-surface-udf-error-messages

**Generated:** 2026-06-16

## Verdict

| Result | Details |
|--------|---------|
| **PASS** | UDF-returned `UdfError` text now reaches the SQL error end-to-end. All offline gates clean; live E2E against Exasol 2025.1.11 confirms the SQL error contains the UDF-supplied `JSON parse error` text. A HIGH-severity FFI allocator bug surfaced in review was fixed before sign-off. |

| Check | Status |
|-------|--------|
| Build | ✓ |
| Tests | ✓ |
| Lint | ✓ |
| Format | ✓ |
| Scenario Coverage | ✓ |
| Manual Tests | ✓ |

## Test Evidence

### Test Results

| Type | Run | Passed | Ignored |
|------|-----|--------|---------|
| Unit (`exasol-udf-sdk`, `exasol-udf-macros`, `exa-udf-runtime`) | 29 | 29 | 0 |
| Integration (`it::db_roundtrip_all_scenarios`, Exasol 2025.1.11) | 1 | 1 | 0 |

Notable unit coverage:
- `exasol-udf-sdk::abi` — `abi_version_and_vtable_layout`, `connect_back_feature_compiles` assert `EXA_UDF_ABI_VERSION == 4`; both `run_stub`s carry the new `error_out` parameter (8 passed).
- `exasol-udf-macros::run_error::run_shim_writes_malloc_backed_error_string_on_user_error` — new regression test asserting code `1`, a non-null malloc-backed string, freeable via C `free`, carrying the error text.
- `exasol-udf-macros` trybuild `dup_entry` snapshot regenerated for the hoisted `__exa_write_c_string` helper.

### Manual Tests

| Test | Result |
|------|--------|
| `EXASOL_DB_SERIES=2025-1 cargo test -p it --features integration,db-2025-1 --test db_roundtrip` → `udf_error_message_reaches_db` passes (SQL error contains `JSON parse error`) | ✓ (57.6s) |
| Recompile-only contract: UDF source unchanged; new `.so`s built against new SDK load and run under the rebuilt ABI-v4 host | ✓ (all 8 UDF scenarios green) |
| v3-ABI `.so` rejection by loader | ✓ (covered by the version gate: `EXA_UDF_ABI_VERSION` 3→4 + existing `LoadedUdf::open` `AbiMismatch` check; mismatch is a clean load-time error, not UB) |

Evidence — both error scenarios ran in the live umbrella test (`--nocapture`):
```
[it] scenario udf_error ok            # udf_error_surfaces_prefix (unchanged)
[it] scenario udf_error_message ok    # udf_error_message_reaches_db (new)
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 57.61s
```

## Tool Evidence

### Linter

```
cargo clippy --all-targets --all-features -- -D warnings
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.69s
(0 warnings)
```

### Formatter

```
cargo fmt --check
FMT CLEAN (no diff)
```

### Build

```
cargo build --release
    Finished `release` profile [optimized] target(s) in 9.93s   (exit 0)
```

## Scenario Coverage

| Domain | Feature | Scenario | Test Location | Test Name | Passes |
|--------|---------|----------|---------------|-----------|--------|
| sdk | udf-sdk | Run shim surfaces UDF error text via an out-pointer parameter | `crates/it/tests/db_roundtrip.rs` | `udf_error_message_reaches_db` | Pass |
| runtime | host-dispatch | Dispatch reads UDF error text from the run out-pointer | `crates/it/tests/db_roundtrip.rs` | `udf_error_message_reaches_db` | Pass |
| integration | db-roundtrip | UDF error message content is surfaced without truncation | `crates/it/tests/db_roundtrip.rs` | `udf_error_message_reaches_db` | Pass |
| sdk | udf-sdk | ABI version bump guards stale `.so`s | `crates/exasol-udf-sdk/src/abi.rs` | `abi_version_and_vtable_layout` | Pass |

## Notes

- **Code-review fix (HIGH, resolved):** The plan literally specified `CString::into_raw`/`from_raw` for the run-error string. Review flagged this as cross-allocator UB: an Option-A precompiled musl `.so` statically links its own Rust global allocator, separate from the host `exaudfclient` binary's, so `into_raw` in one + `from_raw` in the other corrupts the heap on the user-error path. The fix routes the string through the C allocator (`malloc` in the `.so` via a single hoisted `__exa_write_c_string` helper shared with the vs_adapter shim; `libc::free` in the host via the existing `single_call::take_c_string`), matching the convention already documented in `single_call.rs` and fulfilling the plan's stated intent ("one consistent ownership convention"). This deviation from the literal task text is deliberate and correct.
- The in-process E2E test cannot reproduce the original UB (a single test binary shares one allocator); correctness of the fix rests on the structural ownership convention plus the new unit regression test, verified by clean build/clippy across the workspace including the musl-oriented `test-udfs`.
- Final ownership invariant: error C string is `malloc`-allocated by the `.so` and freed with `libc::free` by the host; Rust's global allocator never crosses the boundary. Success (`0`) and panic (`2`) paths leave `*error_out` null.
- The connect-back `last_error`/`take_last_error`/`record_error` plumbing is untouched and still serves the connect-back error path (no dead code introduced).
