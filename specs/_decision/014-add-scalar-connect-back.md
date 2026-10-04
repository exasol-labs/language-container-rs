# Decisions: add-scalar-connect-back

## ADR: Connect-back is fully supported in SCALAR scripts

**ID:** connect-back-fully-supported-in-scalar-scripts
**Plan:** `add-scalar-connect-back`
**Status:** Accepted

### Context

Scalar (`ExactlyOnce`) and set (`Multiple`) UDFs share one run loop in `crates/exa-udf-runtime/src/dispatch.rs`. Connect-back is transport behaviour, not UDF-type behaviour, and the loopback-address cause of the SIGABRT is fixed by using `<container-eth0-ip>:8563`.

### Decision

Connect-back is supported for both `SCALAR` and `SET/EMITS` Rust scripts. The project CLAUDE.md states this positively and has no "never SCALAR" rule. The address rule (`cluster_ip()`, never loopback) and the transaction-conflict rule apply to both types.

### Options Considered

| Option | Verdict |
|--------|---------|
| Support SCALAR as-is, no runtime change | ✓ Chosen |
| Scalar-specific verification step or fast path | ✗ The run loop is shared; adds complexity |
| Keep "never SCALAR" with a footnote | ✗ A self-contradicting rule is noise |

### Consequences

Authors choose `SCALAR` or `SET` by UDF logic alone. The address rule and the `std::process::exit(0)` lifecycle rule apply to both. The `connect-back-scalar` crate is the canonical example and integration fixture.
