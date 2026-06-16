# Tasks: fix-surface-udf-error-messages

## Phase 2: Implementation (Group A — signature + test authoring)
- [x] 2.1 ABI: change `run` signature to take `error_out: *mut *mut c_char`, bump `EXA_UDF_ABI_VERSION` 3→4, update doc + in-file tests (`abi.rs`)
- [x] 2.4 Add E2E scenario `udf_error_message_reaches_db` in `crates/it/tests/db_roundtrip.rs` (asserts SQL error contains `JSON parse error`)

## Phase 2: Implementation (Group B — ABI wiring) [depends on Group A]
- [x] 2.2 Macro shim `__exa_run_shim`: add `error_out` param, write error text on `Err` arm (`exasol-udf-macros`) [expert]
- [x] 2.3 Host dispatch `run_batch` + `LoadedUdf::run`: thread `error_ptr` through, read/free it after non-zero rc (`exa-udf-runtime`) [expert]

## Phase 2.5: Code-review fixes
- [x] 2.5 [HIGH] Route the run-error C string through the C allocator (`malloc`/`libc::free`) instead of Rust `into_raw`/`from_raw`, matching the single-call hook convention — avoids cross-allocator UB in precompiled musl `.so`s [expert]

## Phase 3: Verification
- [x] 3.1 Build (`cargo build --release`)
- [x] 3.2 Lint (`cargo clippy --all-targets --all-features -- -D warnings`)
- [x] 3.3 Format (`cargo fmt --check`)
- [x] 3.4 Unit tests (`cargo test` excluding live-DB E2E) + abi.rs tests assert version 4
- [x] 3.5 E2E `udf_error_message_reaches_db` against Exasol 2025.1.11 (`EXASOL_DB_SERIES=2025-1`)
