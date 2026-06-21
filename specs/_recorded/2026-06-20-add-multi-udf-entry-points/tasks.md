# Tasks: add-multi-udf-entry-points

## Group A: Macro (crates/exasol-udf-macros)
- [x] 1 Parse `name = "..."` attribute
  - [x] 1.1 Add `name: Option<String>` to `Annotations`; parse `name = "literal"`
  - [x] 1.2 Update unknown-key error to mention `name`
- [x] 2 Derive SQL name + namespace every generated symbol [expert]
  - [x] 2.1 Compute `udf_name` (verbatim `name`, else `fn_ident.to_uppercase()`)
  - [x] 2.2 Build suffixed idents via `format_ident!`
  - [x] 2.3 Rewrite schema/vs_adapter/main `quote!` to suffixed idents; remove bare `__exa_udf_entry`
- [x] 3 Update macro unit/trybuild tests
  - [x] 3.1 Update annotation.rs, annotation_typed.rs, run_error.rs, vs_adapter.rs to named symbol
  - [x] 3.2 Repoint dup_entry trybuild to same-name; add distinct-name two-symbol test

## Group B: Runtime loader (crates/exa-udf-runtime)
- [x] 4 Name-parameterized lookup
  - [x] 4.1 `LoadedUdf::open(path, script_name)` builds `__exa_udf_entry_<name>` symbol
  - [x] 4.2 Absent symbol → rebuild-hint error
  - [x] 4.3 Update `Runtime::run` caller to pass `meta.script_name`
- [x] 5 Update loader tests
  - [x] 5.1 loader.rs: named-symbol fixtures, missing-entry + legacy-bare rejection tests
  - [x] 5.2 Update connect_back.rs/main.rs callers of `.open(...)`

## Group C: cargo-exaudf (crates/cargo-exasol-udf)
- [x] 6 Enumerate named entry points [expert]
  - [x] 6.1 validate.rs: enumerate `__exa_udf_entry_*`, validate each vtable
  - [x] 6.2 build.rs: "at least one `__exa_udf_entry_*`" check
  - [x] 6.3 tests/validate.rs: update expectations

## Group D: Fixtures + version + docs
- [x] 7 test-udfs fixtures
  - [x] 7.1 Add `fn annotated_double` to annotated-fixture
  - [x] 7.2 Fix annotated-double self-test to call `__exa_udf_entry_ANNOTATED_DOUBLE()`
- [x] 8 Version bump 0.13.1 → 0.14.0 (+ Cargo.lock)
- [x] 9 Docs: multiple UDFs per .so, `name=`, UPPER_SNAKE rule, rebuild note

## Phase 4: Code Review
- [x] R Review all changed files

## Phase 5: Verification
- [x] V1 cargo build --release
- [x] V2 cargo test
- [x] V3 cargo clippy --all-targets --all-features -- -D warnings
- [x] V4 cargo fmt --check
