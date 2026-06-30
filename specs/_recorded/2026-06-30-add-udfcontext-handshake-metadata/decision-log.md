# Decision Log: add-udfcontext-handshake-metadata

Date: 2026-06-30

## Interview

No clarifying questions were asked. GitHub issue #39 is precise and self-contained: both parts are independent cleanup + additive convenience changes with no feature-flag requirements, and the issue enumerated the exact field set, proto field numbers, and code locations.

## Design Decisions

### [1] Mirror the `memory_limit()` defaulted-accessor pattern for all handshake metadata

- **Decision:** Add each handshake field as a provided (defaulted) `UdfContext` method returning a neutral value, overridden by `HostContextBridge`. No new trait, no feature gate.
- **Alternatives:** A single `handshake_info()` accessor returning a struct (rejected — a struct type would have to cross the `.so` vtable boundary and breaks the additive-default compatibility property); feature-gating behind `connect-back` (rejected — handshake metadata is plain DB context, not a connect-back capability, per CLAUDE.md).
- **Rationale:** `memory_limit()` is the established, ADR-aligned precedent; per-field defaulted accessors keep every existing `UdfContext` impl compiling.
- **Promotes to ADR:** no

### [2] String accessors return owned `String` / `Option<String>`, not borrows

- **Decision:** `database_name`/`database_version`/`script_name`/`script_schema` return owned `String`; `current_user`/`current_schema`/`scope_user` return `Option<String>` with `None` default.
- **Alternatives:** Return `&str` borrowing from the bridge (rejected — lifetime entanglement across the trait and the `.so` boundary); empty string for absent optionals (rejected — loses the proto `optional` present/absent distinction).
- **Rationale:** Owned values cross the dynamic-library vtable boundary safely; `Option` preserves DB semantics.
- **Promotes to ADR:** no

### [3] Delete `UdfMeta::conn_info` but keep `ConnInfo` and `HostEvent::ConnInfo`

- **Decision:** Remove the write-only `conn_info` field and its handshake-loop buffering; retain the `ConnInfo` type and `HostEvent::ConnInfo` event used by the live on-demand `MT_IMPORT` path.
- **Alternatives:** Leave the dead field in place (rejected — write-only state read nowhere is a maintenance hazard and contradicts ADR-018).
- **Rationale:** ADR-018 switched to on-demand per-name credential resolution; the buffered field is a pre-ADR-018 leftover. The type and event remain load-bearing for the live path.
- **Promotes to ADR:** no

## Review Findings

<!-- Populated by speq-implement after code review. -->
