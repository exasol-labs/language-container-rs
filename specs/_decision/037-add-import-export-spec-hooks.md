# Decisions: add-import-export-spec-hooks

## ADR: Every context-taking single-call hook uses one slot shape

**ID:** context-taking-single-call-hook-shape
**Plan:** add-import-export-spec-hooks
**Status:** Accepted

### Context

`generate_sql_for_import_spec` and `generate_sql_for_export_spec` need host state (`script_schema()`, `node_count()`, `connection(name)`), as `virtual_schema_adapter_call` does. Two slot shapes for one concept would split the vtable.

### Decision

`generate_sql_for_import_spec` and `generate_sql_for_export_spec` take `(ctx: *mut c_void, json_spec: *const c_char, result: *mut *mut c_char)`, the shape `virtual_schema_adapter_call` uses. The dispatcher threads one `SingleCallContext` down that path. Every single-call hook that can need host state uses this shape.

### Options Considered

| Option | Verdict |
|--------|---------|
| One shared slot shape for all three hooks | ✓ Chosen |
| Parallel context-carrying slots beside the existing two | ✗ Two shapes for one concept |

### Consequences

One dispatcher path serves all three hooks. The vtable change bumps `EXA_UDF_ABI_VERSION`.
