# Decision Log: add-multi-udf-entry-points

Date: 2026-06-19

## Interview

**Q:** How should the UDF name be declared in the annotation?
**A:** If `name = "MY_UDF"` is not specified, the function name is used and translated to SQL conventions (snake_case → UPPER_SNAKE_CASE). Example: `fn double_it(…)` → SQL name `DOUBLE_IT`. Explicit override: `#[exasol_udf(name = "MY_CUSTOM")]`.

**Q:** How should multiple UDFs be exported from a single .so?
**A:** Per-UDF symbols — each UDF exports `__exa_udf_entry_<NAME>` (no_mangle). Example: `__exa_udf_entry_DOUBLE_IT`, `__exa_udf_entry_TRIPLE_IT`. No registry symbol.

**Q:** What happens to existing single-UDF .so files built with the current macro (one `__exa_udf_entry`)?
**A:** Hard-break — runtime only looks for named symbols. Old `.so` files fail at load time with: `"no entry point found for script 'X'; hint: rebuild with sdk ≥ <next version>"`.

## Design Decisions

### [1] Per-UDF `__exa_udf_entry_<NAME>` symbols, no registry

- **Decision:** Each annotated function exports its own `#[no_mangle]` `__exa_udf_entry_<NAME>` symbol; the loader resolves exactly one by the DB-supplied script name. No registry symbol or name→vtable table is emitted.
- **Alternatives:** A single registry symbol returning a table of (name, vtable) pairs that the loader walks.
- **Rationale:** A direct `dlsym` by script name needs no table format, no allocation, and no new ABI to version. It preserves the existing one-symbol loader shape and lets the linker reject same-name duplicates for free.
- **Promotes to ADR:** yes

### [2] Hard-break the bare `__exa_udf_entry` symbol (no fallback)

- **Decision:** The macro stops emitting the bare `__exa_udf_entry`; the loader never falls back to it. Legacy `.so` files fail with an explicit rebuild-hint error.
- **Alternatives:** Keep emitting the bare symbol and fall back when the named symbol is absent, for backward compatibility.
- **Rationale:** Interview decision. A silent fallback is dangerous once a `.so` carries multiple UDFs (which UDF would the bare symbol mean?). An explicit, actionable error is safer than ambiguous behavior. The project is pre-1.0, so a clean break is acceptable.
- **Promotes to ADR:** yes

### [3] SQL name derived from the function identifier via ASCII `UPPER_SNAKE_CASE`

- **Decision:** Default name = `fn_ident.to_uppercase()` (underscores preserved); `name = "..."` overrides verbatim. The derived name must equal the DB's bare `script_name`.
- **Alternatives:** Always require an explicit `name`; or keep the identifier verbatim (case-sensitive).
- **Rationale:** Exasol object names are upper-cased by default, so `fn double_it` naturally maps to `CREATE SCRIPT DOUBLE_IT`. Zero-config for the common case; `name =` covers quoted/unusual identifiers.
- **Promotes to ADR:** yes

### [4] Reuse the existing `UdfMeta.script_name` from the handshake

- **Decision:** Thread the already-populated `UdfMeta.script_name` (from proto `exascript_info.script_name`, field 3 — the bare object name) into `LoadedUdf::open` rather than parsing a name from `%udf_object` or adding a proto field.
- **Alternatives:** Add a new proto field; or derive the name from the `.so` filename / `%udf_object` path.
- **Rationale:** The field already exists and is the authoritative object name; no protocol or parsing change is needed. `script_schema` (field 13) is deliberately ignored — only the bare name participates in the symbol.
- **Promotes to ADR:** no

### [5] CLI (`validate`/`build`) enumerates `__exa_udf_entry_*` symbols

- **Decision:** Because the CLI has no DB and no script name, `validate` discovers all exported `__exa_udf_entry_*` symbols and checks each vtable's ABI/fingerprint; `build` verifies at least one named entry exists.
- **Alternatives:** Add a `--name` flag to the CLI.
- **Rationale:** Enumeration validates every embedded UDF in one pass with no extra author input, and naturally rejects legacy single-symbol `.so` files (no `__exa_udf_entry_*` match).
- **Promotes to ADR:** no

### [6] Per-UDF copy of the `__exa_write_c_string_<NAME>` helper

- **Decision:** Emit a namespaced copy of the malloc-backed C-string helper for each UDF rather than one shared module-level helper.
- **Alternatives:** Emit one shared `__exa_write_c_string` referenced by all UDFs' shims.
- **Rationale:** A shared helper reintroduces a fixed-name symbol and the duplicate-symbol problem the whole change exists to avoid (two annotations would both define it). Duplicating a tiny unsafe helper is cheap and keeps each UDF's generated code fully self-contained.
- **Promotes to ADR:** no

### [7] MINOR version bump 0.13.1 → 0.14.0 for a breaking change

- **Decision:** Bump the workspace version (and the in-sync `exasol-udf-sdk` pin) to `0.14.0`.
- **Alternatives:** PATCH bump to `0.13.2`.
- **Rationale:** This breaks author artifacts (rebuild required). Pre-1.0, a MINOR bump is the conventional signal for a breaking change; a PATCH would under-signal it.
- **Promotes to ADR:** no

## Review Findings

<!-- Populated by speq-implement after code review. -->
