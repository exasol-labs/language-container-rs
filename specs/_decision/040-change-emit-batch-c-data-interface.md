# Decisions: change-emit-batch-c-data-interface

## ADR: `emit_batch` crosses the `.so` boundary through the Arrow C Data Interface

**ID:** emit-batch-arrow-c-data-interface
**Plan:** `change-emit-batch-c-data-interface`
**Status:** Accepted

### Context

The UDF and the host link separate `arrow` copies, so no Rust Arrow type can cross the `.so` boundary. The C Data Interface is Arrow's ABI-stable in-process exchange.

### Decision

`emit_batch` exports the batch through the Arrow C Data Interface. The host takes ownership of the exported array, imports it without copying or validating its buffers, and packs it into the wire's type blocks.

### Options Considered

| Option | Verdict |
|--------|---------|
| Arrow C Data Interface | ✓ Chosen |
| Arrow IPC bytes | ✗ Copies every buffer on both sides; batch emit was slower than row emit on wide outputs |
| Wire-form blocks packed in the SDK | ✗ Removes the same copies but moves the packer and formatters into every UDF |

### Consequences

`EXA_UDF_ABI_VERSION` goes to 12 and every UDF is rebuilt. Neither side depends on `arrow-ipc`. Arrays a UDF built with unchecked constructors reach the database as built.
