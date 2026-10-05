# Decisions: add-multi-udf-entry-points

## ADR: Per-UDF `__exa_udf_entry_<NAME>` symbols, no registry

**ID:** per-udf-entry-point-symbols-no-registry
**Plan:** `add-multi-udf-entry-points`
**Status:** Accepted

### Context

One `.so` must host multiple UDFs, so each annotated function needs its own ABI entry point.

### Decision

Each annotated function exports its own `#[unsafe(no_mangle)]` `__exa_udf_entry_<NAME>` symbol. The loader resolves one symbol by the DB-supplied `script_name`. No registry symbol or name-to-vtable table exists.

### Options Considered

| Option | Verdict |
|--------|---------|
| Per-UDF `__exa_udf_entry_<NAME>` symbols | ✓ Chosen: direct `dlsym`, no table ABI, linker rejects duplicates |
| Single registry symbol returning a name→vtable table | ✗ Needs a table format, allocation and a versioned ABI |

### Consequences

The loader does one `dlsym` per session. A same-name duplicate in one crate is a link-time error.

## ADR: Hard-break the bare `__exa_udf_entry` symbol, no fallback

**ID:** hard-break-bare-udf-entry-symbol
**Plan:** `add-multi-udf-entry-points`
**Status:** Accepted

### Context

A bare `__exa_udf_entry` fallback is ambiguous once a `.so` carries multiple UDFs.

### Decision

The macro does not emit `__exa_udf_entry`, and the loader never falls back to it. A `.so` without the named symbol fails at load with an error that names the script and the missing symbol and tells the author to rebuild with a current SDK.

### Options Considered

| Option | Verdict |
|--------|---------|
| Remove the bare symbol, fail with a rebuild hint | ✓ Chosen: the project is pre-1.0 and the error is actionable |
| Keep the bare symbol as fallback | ✗ Loads the wrong UDF in a multi-UDF `.so` |

### Consequences

The rebuild-hint error reaches the database through the protocol close path with the `F-UDF-CL-RUST-` prefix.

## ADR: SQL name derived from function identifier via ASCII UPPER_SNAKE_CASE

**ID:** sql-name-derived-upper-snake-case
**Plan:** `add-multi-udf-entry-points`
**Status:** Accepted

### Context

Each `#[exasol_udf]` function needs an SQL name that suffixes its symbols and matches the DB's `script_name`.

### Decision

The default SQL name is `fn_ident.to_uppercase()`, underscores preserved, matching Exasol's identifier uppercasing. A `name = "..."` attribute overrides it verbatim. The name equals the bare object name the database sends as `script_name`; `script_schema` is not part of the symbol.

### Options Considered

| Option | Verdict |
|--------|---------|
| Uppercase the identifier; `name=` overrides | ✓ Chosen: `fn double_it` matches `CREATE SCRIPT DOUBLE_IT` |
| Always require `name = "..."` | ✗ Boilerplate for the common case |
| Keep the identifier verbatim | ✗ Does not match Exasol's uppercased names |

### Consequences

`name = "..."` covers quoted or non-`UPPER_SNAKE_CASE` names.
