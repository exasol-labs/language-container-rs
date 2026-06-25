# Decision Log: fix-abi-feature-safety

Date: 2026-06-25

## Background

This plan was rescoped from `add-arrow-cdata-fast-path`. An end-to-end emit-throughput
benchmark (Rust SLC vs native Python3) disproved the premise of the Arrow C Data
Interface work: Arrow IPC ser/deser is only 2–9% of `emit_batch`'s cost (the dominant
cost is per-cell proto string-block encoding, shared by both paths), and Rust emit is
already ~4.5–5.9× faster than native Python3. Issue #29's own gate ("only worth it if
profiling shows IPC dominates") is not met, so the entire Arrow C Data Interface scope
(`arrow-ffi`, `query_arrow_ffi`, `emit_batch_ffi`) is dropped.

The benchmark also surfaced issue **#31** (UdfContext vtable feature-skew silently drops
`emit_batch` rows). This plan now targets the two real safety defects: **#26** and **#31**.

## Design Decisions

### [1] Remove `query_arrow`; no Arrow accessor replacement (#26)

- **Decision:** Drop `ExaConnection::query_arrow` (returned `Vec<arrow::RecordBatch>`).
  Make `query_for_each` (`Vec<Value>` callback) the required streaming method; `query`
  defaults to it. Do NOT add a `query_arrow_ffi` / C Data Interface replacement.
- **Alternatives:** `#[deprecated]`; gate behind a feature; replace with `query_arrow_ffi`.
- **Rationale:** `query_arrow` is an unconditional memory-safety footgun (#26 — `TypeId`
  mismatch across two static `arrow` copies → silent wrong values). The benchmark showed
  Arrow buys nothing here, so `Vec<Value>` (already safe and ergonomic) is the answer; no
  replacement API is warranted.
- **Promotes to ADR:** yes

### [2] Make the UdfContext trait-object vtable feature-independent (#31)

- **Decision:** Remove every `#[cfg(feature=...)]` from `UdfContext` method declarations;
  declare `cluster_ip`/`connection`/`connect_back`/`emit_record_batch_ipc` unconditionally
  with `Unimplemented` defaults, so the `dyn UdfContext` vtable layout is identical in all
  feature configurations.
- **Alternatives:** Encode the feature set in the ABI fingerprint so the loader rejects a
  mismatched `.so` (detect-only); a separate `#[repr(C)]` context vtable (heavy).
- **Rationale:** The `.so`↔host context crosses as a Rust trait object whose vtable is
  declaration-order-dependent; feature-gated methods shift slots and the fingerprint does
  not encode features → silent misdispatch. A feature-independent trait eliminates the
  skew class entirely. Fingerprint-feature encoding remains a possible defense-in-depth
  follow-up but is not needed once the layout is stable.
- **Promotes to ADR:** yes

### [3] Coupling: #26 enables #31

- **Decision:** Land #26 and #31 together. Removing `query_arrow` makes `ExaConnection`
  arrow-free, which lets the `connect_back` module + `ConnectionObject`/`ExaConnection`
  compile unconditionally (no SDK `connect-back` feature) — the prerequisite for declaring
  `UdfContext::connection`/`connect_back` unconditionally.
- **Rationale:** Without #26, the connect-back types still need `arrow` and a feature gate,
  so the trait could not be made feature-independent cleanly.
- **Promotes to ADR:** no

### [4] Bump EXA_UDF_ABI_VERSION 4 → 5

- **Decision:** Increment the ABI version so a `.so` built against the old, feature-
  dependent layout fails the loader's existing version check with `AbiMismatch`.
- **Alternatives:** Leave it (rely on the new layout being compatible).
- **Rationale:** The `dyn UdfContext` layout changes; a stale `.so` must fail loudly, not
  misdispatch. The `#[repr(C)] ExaUdfVTable` field order is unchanged so the version field
  stays readable.
- **Promotes to ADR:** no

### [5] Remove the SDK `connect-back` feature (keep the runtime's)

- **Decision:** Delete the `connect-back` cargo feature from `exasol-udf-sdk` (types are
  now unconditional and arrow-free). The runtime keeps its own `connect-back` feature to
  gate the heavy deps (exarrow-rs/tokio/rustls) and the `RuntimeExaConnection` impl.
- **Rationale:** A vestigial SDK feature only re-introduces the vtable-skew hazard. The
  heavy deps that genuinely need gating live in the runtime, not the SDK.
- **Promotes to ADR:** no

## Review Findings

<!-- Populated by speq-implement after code review. -->
