# Decisions: 2026-06-11-vs-adapter-and-single-call-connect-back

## ADR: Row-major type-block packing with NULL cells skipping the type block

**ID:** row-major-type-block-packing-null-cells
**Plan:** `2026-06-11-vs-adapter-and-single-call-connect-back`
**Status:** Accepted

### Context

The Exasol wire format is row-major with no NULL slots. A placeholder entry for a NULL cell shifts every later value in its type block into the wrong column.

### Decision

`EmitBuffer::to_proto` and `InputRowSet::from_proto` order each type block row-major (row, then column). A NULL cell sets only the null-bitmap and adds no slot to the type block. Per-type cursors advance only on non-null cells. Output values are packed by declared column `ExaType`, not by runtime `Value` variant (a `Value::Int64` in a `Numeric` column is stringified into the string block).

### Options Considered

| Option | Verdict |
|--------|---------|
| Row-major, NULL cells skip the slot | ✓ Chosen, matches the C++ reference |
| Column-major with `n_rows` placeholders per column | ✗ Corrupts values after a NULL |
| Row-major with NULL placeholders | ✗ The placeholder causes the corruption |

### Consequences

None beyond the decision.
