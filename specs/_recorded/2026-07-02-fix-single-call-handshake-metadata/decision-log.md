# Decision Log: fix-single-call-handshake-metadata

Date: 2026-07-02

Fixes GitHub issue exasol-labs/language-container-rs#41. Downstream consumer impact tracked in exasol-labs/lakehouse-engine-rs#43.

## Interview

**Q:** Issue #41 lists 6 missing accessors on `SingleCallContext` (`node_count`, `node_id`, `session_id`, `statement_id`, `vm_id`, `memory_limit`) plus the string fields `HostContextBridge` also overrides. How far should the fix reach?
**A:** Full parity with `HostContextBridge` — override every handshake accessor `SingleCallContext` is missing (all numeric IDs + `memory_limit` + all string/optional fields: `database_name`, `database_version`, `script_name`, `script_schema`, `current_user`, `current_schema`, `scope_user`), reusing the existing `HandshakeMeta` struct/pattern and its `From<&UdfMeta>` impl from PR #40. Do NOT do the minimal `node_count`/`node_id`-only fix — go all the way to parity.

**Q:** No test currently proves the DB sends real `node_count`/etc. through the single-call/adapter path specifically (existing single-call IT scenarios only check the script loads, never invoke the hook against a live ctx with real assertions). How should the plan close that gap?
**A:** Real `CREATE VIRTUAL SCHEMA` round-trip — extend the `single-call-fixture` `virtual_schema_adapter_call` shim to read live metadata off the raw `ctx` pointer (via the double-indirection `&mut &mut dyn UdfContext` ABI contract) and echo `node_count`/`node_id` (and ideally the other fixed fields) back. Add a new IT scenario in `crates/it/tests/db_roundtrip.rs` doing a genuine `CREATE VIRTUAL SCHEMA ... USING <script>` round-trip that asserts the returned values are live (non-zero/non-empty), mirroring `handshake_metadata_udf_emits_session_and_node`. This must exercise the actual `SC_FN_VIRTUAL_SCHEMA_ADAPTER_CALL` path — the exact call path the linked lakehouse-engine-rs#43 bug goes through — not the weaker "script loads" check.

## Design Decisions

### [1] Full accessor parity, reusing `HandshakeMeta`

- **Decision:** Give `SingleCallContext` a `handshake: HandshakeMeta` field and override all 13 handshake accessors as one-line passthroughs, identical to `HostContextBridge`. Reuse the existing `HandshakeMeta` struct and its `From<&UdfMeta>` impl.
- **Alternatives:** Minimal `node_count`/`node_id`-only fix (rejected per interview — re-litigates later for other consumers); a new single-call-specific meta type (rejected — dead duplication of an existing `Clone` struct).
- **Rationale:** Both single-call and streaming paths should behave identically for handshake metadata; parity is the least-surprising, lowest-maintenance shape.
- **Promotes to ADR:** no

### [2] Handshake accessors are not feature-gated

- **Decision:** The new `SingleCallContext` overrides and the `handshake` constructor param are unconditional (not behind `connect-back`), matching `HostContextBridge` and the `sdk/udf-sdk` spec.
- **Alternatives:** Gate behind `connect-back` alongside the existing `conn_requester` field.
- **Rationale:** Handshake metadata is plain DB-supplied context, not a connect-back capability — this rule is already established project-wide (CLAUDE.md, PR #40).
- **Promotes to ADR:** no

### [3] IT observability via the deliberate-error echo channel

- **Decision:** The `single-call-fixture` adapter shim reads live metadata off `ctx` and returns rc=1 embedding the values in the error out-pointer; the new IT scenario issues a real `CREATE VIRTUAL SCHEMA`, expects it to fail, and asserts the surfaced error text contains a non-zero `node_count`.
- **Alternatives:** Return a valid `createVirtualSchema` response encoding metadata in virtual table/column identifiers, then query the created schema's catalog to read the values back.
- **Rationale:** The single-call error out-pointer path is already proven to surface hook text; it is version-robust and asserts the exact bug directly. A valid schema-metadata response is brittle across DB versions and forces numeric-in-identifier encoding/parsing to read values back. The trade-off is that the schema is not actually created — acceptable because the goal is proving live metadata reaches the adapter hook, not exercising schema creation itself.
- **Promotes to ADR:** no

### [4] Version bump is PATCH, not MINOR

- **Decision:** Bump 0.20.0 → 0.20.1 (PATCH).
- **Alternatives:** MINOR bump.
- **Rationale:** The `UdfContext` trait surface does not change — the accessors already exist from PR #40; this only wires an already-received-but-discarded `_meta` into the single-call context. That is a bug fix, which is PATCH under SemVer. CLAUDE.md still requires bumping the version and the pinned `exasol-udf-sdk` entry and committing `Cargo.lock` on every change.
- **Promotes to ADR:** no

### [5] IT doubles as the "DB populates exascript_info for adapter calls" confirmation

- **Decision:** Treat the new integration scenario as the confirmation mechanism for the issue's "not yet verified" question (whether the engine populates `exascript_info.node_count` for VS-adapter single-call invocations, vs only for data-UDF streaming sessions).
- **Alternatives:** Block on finding DB documentation first.
- **Rationale:** Per the interview, if documentation can't confirm it the test is the confirmation. If the DB does not populate the field for adapter calls, the IT fails with `node_count=0` — a legitimate, informative finding rather than a silent gap.
- **Promotes to ADR:** no

## Review Findings

<!-- Populated by speq-implement after code review. -->
