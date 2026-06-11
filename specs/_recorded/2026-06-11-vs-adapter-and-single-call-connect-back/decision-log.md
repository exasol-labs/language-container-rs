# Decision Log: 2026-06-11-vs-adapter-and-single-call-connect-back

Date: 2026-06-11

## Interview

**Q:** What is the scope of this plan?
**A:** Four capabilities already implemented in the working tree: (1) connect-back transaction control API (`begin`/`commit`/`rollback` on `ExaConnection`), (2) `vs_adapter` macro annotation with ABI v2→v3, (3) single-call `MT_DONE` protocol fix (the C++ `MT_RUN→MT_CALL→dispatch→MT_RETURN(ack)→MT_DONE→MT_CLEANUP` loop), (4) rowset overhaul to row-major packing with no NULL placeholders. The plan is retroactive — implementation already exists, the spec deltas must describe it accurately for `/speq:record`.

**Q:** Should the plan propose alternatives or new designs?
**A:** No. Reverse-engineer the specs from the actual diff. Do not propose alternatives to what is already implemented.

**Q:** What version bump is recommended?
**A:** 0.4.0→0.5.0. The ABI break (vtable slot signature change) dominates pre-1.0 semver. The orchestrator handles the Cargo.toml bump and commit.

**Q:** Are there threshold concerns?
**A:** The changed features (sdk/udf-sdk, runtime/host-dispatch) are approaching the scenario limit. Each feature receives 2–3 new scenarios. See escalation note below.

## Design Decisions

### [1] ABI version bump 2→3 for virtual_schema_adapter_call signature change

- **Decision:** Increment `EXA_UDF_ABI_VERSION` from 2 to 3. The `virtual_schema_adapter_call` vtable slot changes from a 2-argument signature `(json_arg, result)` to a 3-argument signature `(ctx, json_arg, result)`.
- **Alternatives:** Keep ABI v2 and add a separate slot for the context-bearing variant; use a struct-based calling convention.
- **Rationale:** The slot signature change is a binary incompatibility. Any `.so` compiled against ABI v2 that happened to wire this slot would be called with an extra argument, producing undefined behavior. Incrementing the version makes the loader reject the artifact with a clear error at load time rather than silently invoking the wrong signature. Adding a parallel slot would bloat the vtable and complicate dispatch for no gain.
- **Promotes to ADR:** yes

### [2] Row-major type-block packing with NULL cells skipping the type block

- **Decision:** `EmitBuffer::to_proto` and `InputRowSet::from_proto` use row-major ordering within each type block (row then column), and NULL cells do not push any placeholder slot — only the null-bitmap is updated.
- **Alternatives:** Column-major with `n_rows` placeholder entries per column (the prior implementation); row-major with placeholders.
- **Rationale:** The prior column-major implementation produced silently wrong values when NULL cells appeared: a NULL in column 2 would still write a placeholder into the string block, causing all subsequent string values for that row to land in the wrong column position. Exasol's wire format is row-major with no NULL slots. The fix eliminates the placeholder entirely, using per-type running cursors that advance only on non-null cells. This matches the confirmed C++ reference behavior.
- **Promotes to ADR:** yes

### [3] Default begin/commit/rollback on ExaConnection rather than a separate trait

- **Decision:** Add `begin`, `commit`, and `rollback` as default methods to the existing `ExaConnection` trait, each returning `Err(UdfError::Unimplemented(...))`.
- **Alternatives:** Add a separate `TransactionalExaConnection : ExaConnection` trait; require implementors to provide the methods.
- **Rationale:** Default methods preserve backward compatibility for all existing `ExaConnection` implementations (test mocks, unit doubles). UDF authors who do not need transaction control get a no-change compilation. The Unimplemented default follows the same convention already used for `cluster_ip`/`connection`/`connect_back` on `UdfContext`, keeping the API surface consistent.
- **Promotes to ADR:** no

### [4] SingleCallContext as a UdfContext implementation for VS adapter calls

- **Decision:** Introduce `SingleCallContext<'a>` implementing `UdfContext` for the single-call dispatch path. Data methods (`get`, `emit`, `next`) return `Unimplemented`; connect-back methods delegate to an on-demand `ConnRequester` closure that drives MT_IMPORT over the idle ZMQ socket.
- **Alternatives:** Pass a null/stub context pointer and ignore it; reuse `HostContextBridge`.
- **Rationale:** The VS adapter receives `&mut dyn UdfContext` by the ABI contract, so a concrete implementation is required. `HostContextBridge` is tightly coupled to the scalar/set run loop and carries row-iteration state that is meaningless in single-call mode. A dedicated `SingleCallContext` is minimal and correct. The MT_IMPORT exchange is safe because the dispatch loop is blocked waiting for the hook to return — the socket is not concurrently accessed (the same reasoning already documented for `connection()` in `runtime/connect-back`).
- **Promotes to ADR:** no

### [5] SingleCallAck as a new HostEvent variant rather than re-using Done

- **Decision:** Add `HostEvent::SingleCallAck` to represent the DB's `MT_RETURN` acknowledgement in single-call mode.
- **Alternatives:** Reuse `HostEvent::Done`; handle silently inside the protocol state machine.
- **Rationale:** The DB echoes `MT_RETURN` (not `MT_DONE`) as the ack in single-call mode. Surfacing it as a distinct variant prevents ambiguity in the dispatch loop and makes the protocol state machine's behavior explicit and testable. Handling it silently inside the state machine would hide observable behavior and make unit tests impossible.
- **Promotes to ADR:** no

### [6] Value-to-block-string conversion for declared-type mismatch in EmitBuffer

- **Decision:** When the declared `ExaType` for a column is a string-block type (String, Numeric, Timestamp, Date) but the runtime `Value` variant is numeric (`Int64`, `Int32`, `Double`), stringify the value using `to_string()` and write it into the string block.
- **Alternatives:** Return an error on type mismatch; coerce silently but only for specific pairs.
- **Rationale:** A connect-back `SELECT` may return a DECIMAL column as `Value::Int64` (the exarrow-rs Arrow decoding), but the `EMITS` schema declares it as `Numeric` (string block). The UDF author has no control over this variance. Silent stringification is the correct behavior and matches how Exasol stores DECIMAL/NUMERIC values in the wire format.
- **Promotes to ADR:** no

## Review Findings

<!-- Populated by speq-implement after code review. -->
