# Decisions: add-multi-udf-entry-points

## ADR: Per-UDF `__exa_udf_entry_<NAME>` symbols, no registry

**ID:** per-udf-entry-point-symbols-no-registry
**Plan:** `add-multi-udf-entry-points`
**Status:** Accepted

### Context

Each annotated function in a UDF crate needed its own unique ABI entry point so that one `.so` could host multiple UDFs. Two approaches were considered: emitting a single registry symbol that returns a name-to-vtable table, or emitting one `#[no_mangle]` symbol per UDF suffixed with the SQL name derived from the function identifier.

### Decision

Each annotated function exports its own `#[unsafe(no_mangle)]` `__exa_udf_entry_<NAME>` symbol. The loader resolves exactly one by the DB-supplied `script_name`. No registry symbol or name-to-vtable table is emitted.

### Options Considered

| Option | Verdict |
|--------|---------|
| Per-UDF `__exa_udf_entry_<NAME>` symbols | ✓ Chosen — a direct `dlsym` by script name needs no table format, no allocation, and no new ABI to version; the linker rejects same-name duplicates for free |
| Single registry symbol returning a name→vtable table | ✗ Rejected — requires a table format, allocation, and a new registry ABI to version; does not leverage linker duplicate detection |

### Consequences

One `.so` may export many UDFs, each addressable by the SQL script name the database sends in the handshake. The loader shape is unchanged — it still performs a single `dlsym` per session. A same-name duplicate in one crate is a link-time error, not a silent wrong-UDF selection.

## ADR: Hard-break the bare `__exa_udf_entry` symbol — no fallback

**ID:** hard-break-bare-udf-entry-symbol
**Plan:** `add-multi-udf-entry-points`
**Status:** Accepted

### Context

The macro previously emitted a bare `__exa_udf_entry` symbol (no suffix). Removing it breaks all `.so` artifacts compiled against SDK < 0.14.0. Two options were considered: maintain backward compatibility via a fallback to the bare symbol, or hard-break with an explicit rebuild-hint error.

### Decision

The macro stops emitting `__exa_udf_entry`. The loader never falls back to it. Legacy `.so` files fail at load time with `no entry point found for script '<NAME>'; hint: rebuild with sdk >= 0.14.0`.

### Options Considered

| Option | Verdict |
|--------|---------|
| Hard-break: remove bare symbol; clear rebuild-hint error | ✓ Chosen — interview decision; a silent fallback is dangerous once a `.so` carries multiple UDFs (which UDF would the bare symbol mean?); an actionable error is safer than ambiguous behavior; the project is pre-1.0 so a clean break is acceptable |
| Keep bare symbol; fall back when named symbol absent | ✗ Rejected — would silently load the wrong UDF in a multi-UDF `.so`; masks author error |

### Consequences

All `.so` artifacts built against SDK < 0.14.0 must be rebuilt. The rebuild-hint error message is surfaced through the protocol close path with the `F-UDF-CL-RUST-` prefix. The MINOR version bump (ADR-047) signals the breaking change.

## ADR: SQL name derived from function identifier via ASCII UPPER_SNAKE_CASE

**ID:** sql-name-derived-upper-snake-case
**Plan:** `add-multi-udf-entry-points`
**Status:** Accepted

### Context

Each `#[exasol_udf]`-annotated function needed an SQL entry-point name to suffix its generated symbols and match the DB's `script_name`. Three derivation options were considered: always require an explicit `name = "..."` attribute, keep the identifier verbatim (case-sensitive), or derive from the function identifier by uppercasing.

### Decision

The default SQL name is `fn_ident.to_uppercase()` (underscores preserved), matching Exasol's default identifier uppercasing. A `name = "..."` attribute overrides the derived name verbatim.

### Options Considered

| Option | Verdict |
|--------|---------|
| Derive via ASCII `UPPER_SNAKE_CASE` from fn identifier; `name=` overrides | ✓ Chosen — `fn double_it` → `DOUBLE_IT` naturally matches `CREATE SCRIPT DOUBLE_IT`; zero-config for the common case; `name=` covers quoted or unusual identifiers |
| Always require explicit `name = "..."` | ✗ Rejected — unnecessary boilerplate for the common case where the function name matches the SQL script name |
| Keep identifier verbatim (case-sensitive) | ✗ Rejected — Exasol object names are upper-cased by default; `fn double_it` would not match `CREATE SCRIPT DOUBLE_IT` without quoting |

### Consequences

Authors annotating `fn double_it` get `__exa_udf_entry_DOUBLE_IT` for free. The `name = "..."` attribute is the escape hatch for quoted identifiers or any name that does not follow `UPPER_SNAKE_CASE`. The derived name must equal the bare object name the database sends as `script_name`; `script_schema` is not part of the symbol.
