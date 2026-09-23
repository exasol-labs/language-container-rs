# Decisions: add-import-export-spec-hooks

## ADR: Every context-taking single-call hook uses one slot shape

**ID:** context-taking-single-call-hook-shape
**Plan:** add-import-export-spec-hooks
**Status:** Accepted

### Context

`generate_sql_for_import_spec` and `generate_sql_for_export_spec` need host state
to build a `SELECT`: `script_schema()`, `node_count()`, and `connection(name)`.
ADR `abi-version-bump-2-3-vs-adapter-call` gave `virtual_schema_adapter_call` a
context-carrying slot for the same reason. A second, incompatible slot shape for
the two new hooks would leave two shapes for one concept in the vtable.

### Decision

`generate_sql_for_import_spec` and `generate_sql_for_export_spec` take
`(ctx: *mut c_void, json_spec: *const c_char, result: *mut *mut c_char)`, the
shape `virtual_schema_adapter_call` already uses, and the dispatcher threads one
`SingleCallContext` down that single path. This extends the vs-adapter ADR from
one context-carrying slot to every hook that can need host state, fixing the
shape for future hooks.

### Options Considered

| Option | Verdict |
|--------|---------|
| Reuse the vs-adapter slot shape for both new hooks | ✓ Chosen — one shape for one concept, and the SDK version bump already forces downstream rebuilds |
| Add parallel context-carrying slots beside the existing two, preserving ABI 9 | ✗ Rejected — two shapes for one concept |

### Consequences

`EXA_UDF_ABI_VERSION` goes `9 → 10`. `invoke_vs_adapter_call` generalizes to serve
all three hooks.
