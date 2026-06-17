# Decision Log: add-scalar-connect-back

Date: 2026-06-17

## Interview

**Q:** The decision-log shows the SIGABRT that motivated the "never SCALAR" rule was already fixed (localhost→eth0), and SET connect-back now passes as a hard assertion. How should the plan treat the root-cause/fix?

**A:** Verify live first. Run a Rust SCALAR connect-back against the live container; only design a code fix if it actually crashes.

## Design Decisions

### [1] No runtime code change required — the run loop already shares scalar and set dispatch

- **Decision:** The plan makes zero runtime code changes. `dispatch.rs` drives scalar (`ExactlyOnce`) and set (`Multiple`) UDFs through one identical run loop; connect-back (MT_IMPORT exchange, external session) is transport-layer behaviour that runs identically in both paths. The live spike confirmed this empirically.
- **Alternatives:** Add a scalar-specific connect-back verification step or a dedicated fast-path. Rejected — unnecessary complexity with no correctness benefit.
- **Rationale:** The spike returned `42` with no crash. There is nothing to fix. The plan's job is to document verified reality, not introduce code.
- **Promotes to ADR:** yes

### [2] "Never SCALAR" rule in CLAUDE.md is removed, not softened

- **Decision:** Replace the restrictive "never SCALAR (SCALAR → SIGABRT mid-execution)" line with a positive, accurate statement: both SCALAR and SET scripts support connect-back; the historical SIGABRT was caused by the loopback address (fixed ADR-029), not by the UDF type.
- **Alternatives:** Keep the rule with a footnote "actually works since ADR-029." Rejected — a rule with a footnote that contradicts the rule is noise; the rule itself should be authoritative.
- **Rationale:** Future authors read the Connect-back section to know what is and is not allowed. A stale prohibition will cause them to write less capable UDFs unnecessarily.
- **Promotes to ADR:** no

### [3] Proof chain captured as ADR-040

- **Decision:** A new ADR (ADR-040) in `specs/decision-log.md` records: Python3 SCALAR spike, Rust SCALAR IT run (all 15 scenarios green), code inspection confirming the shared run loop, and the root-cause chain (loopback address → fixed ADR-029 → "never SCALAR" rule became stale).
- **Alternatives:** No ADR — treat as a minor docs cleanup. Rejected — without a permanent record, the restriction will inevitably be re-introduced by a future planner who sees the old commit history and not the spike output.
- **Rationale:** Architectural decisions that prevent re-litigation of a settled question belong in the ADR log. Future maintainers must be able to find the proof in one place.
- **Promotes to ADR:** yes

### [4] Spec delta scope: three features, one new scenario each

- **Decision:** Add one new scenario to each of `integration/connect-back`, `examples/test-udfs`, and `runtime/dispatch-run-loop`. No existing scenarios are modified or removed.
- **Alternatives:** Single delta in `integration/connect-back` only. Rejected — the `examples/test-udfs` feature should catalogue `connect-back-scalar` as a first-class example, and `runtime/dispatch-run-loop` needs an explicit normative statement that connect-back is not SET-only.
- **Rationale:** All three features are affected by the verified behaviour; each needs a spec statement that can be checked off against the existing IT evidence.
- **Promotes to ADR:** no

## Review Findings

<!-- Significant code-review findings that changed implementation direction. -->
<!-- Populated by speq-implement after code review. -->
