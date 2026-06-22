# Decision Log: remove-sdk-dead-public-api

Date: 2026-06-21

## Interview

**Q:** Are all three removal groups in scope — the four unimplemented stubs (`column_name`, `column_type`, `column_index`, `reset`), the `column_count` alias, and `TryFrom<f64> for Decimal`?
**A:** Yes, all three are confirmed in scope.

**Q:** Is the `exaudfclient` `usage()` return-type change (`String` → `&'static str`) in scope?
**A:** Yes, but it is the ONLY launcher change. The `Exit` struct, the endpoint clone, and the `std::process::exit(0)` path MUST NOT be touched.

**Q:** What version should this bump to?
**A:** `0.14.0` → `0.15.0` (MINOR bump under pre-1.0 SemVer to signal a breaking change). Not `1.0.0`. Merging to main will auto-tag and release `v0.15.0` via CI.

**Q:** Should `ExaType` and its `precision`/`scale`/`size` fields be kept?
**A:** Yes — they are populated from proto metadata and documented. Only `TryFrom<f64> for Decimal` is removed, not `ExaType` itself.

**Q:** Which callers of the removed methods need updating?
**A:** Zero in-repo callers exist. Only one doc line (`docs/writing-a-udf.md` ~line 185) references `ctx.column_type(col)` and MUST be updated.

## Design Decisions

### [1] Remove stubs rather than deprecate

- **Decision:** Delete the five never-implemented `UdfContext` methods outright (no `#[deprecated]` gate, no grace-period release). Under pre-1.0 SemVer a MINOR bump (`0.15.0`) is sufficient to communicate a breaking change.
- **Alternatives:** Mark `#[deprecated]` in a PATCH release first and remove in a follow-up MINOR. Rejected because there are zero known external users (crate is pre-1.0 and has only been published for one release cycle), and a two-step removal adds unnecessary ceremony.
- **Rationale:** Dead stubs that only ever return `UdfError::Unimplemented` mislead UDF authors into calling methods that cannot work. Removing them cleanly is less surprising than a deprecated API that always errors at runtime.
- **Promotes to ADR:** no

### [2] Remove `TryFrom<f64> for Decimal`

- **Decision:** Delete `impl TryFrom<f64> for Decimal` and its dedicated test assertions. The wire path uses only `TryFrom<&str>`.
- **Alternatives:** Keep as a convenience conversion. Rejected because `f64 → Decimal` is inherently lossy (floating-point representation; `format!("{value}")` produces a platform-specific number of decimal places), and no caller in the codebase uses it.
- **Rationale:** A conversion that appears lossless but silently degrades precision for values like `0.1` is an API footgun. Removing it forces callers to parse the canonical string representation from the wire.
- **Promotes to ADR:** no

### [3] `usage()` return type: `&'static str` not `String`

- **Decision:** Change the private `usage()` function in `exaudfclient/src/main.rs` to return `&'static str`.
- **Alternatives:** Leave as `String`. Rejected because the allocation is pure overhead for a string literal that never changes.
- **Rationale:** Zero-cost refactor with identical observable behavior; confirms the `Exit` lifecycle path and `std::process::exit(0)` are untouched.
- **Promotes to ADR:** no

### [4] Version bump target: 0.15.0

- **Decision:** Bump workspace `version` to `0.15.0` and the `exasol-udf-sdk` pin in `[workspace.dependencies]` to match.
- **Alternatives:** `1.0.0` (stable API signal), `0.14.1` (PATCH). Both rejected — `1.0.0` overstates stability; PATCH would misrepresent a breaking change under pre-1.0 SemVer.
- **Rationale:** Pre-1.0 SemVer convention: MINOR bump = may break the public API. `0.15.0` is the smallest version number that communicates the break correctly while staying in the pre-1.0 series.
- **Promotes to ADR:** no

## Review Findings

<!-- Populated by speq-implement after code review. -->
