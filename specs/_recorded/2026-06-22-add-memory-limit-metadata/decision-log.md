# Decision Log: add-memory-limit-metadata

Date: 2026-06-22

## Interview

**Q:** What value needs to be surfaced, and to whom?
**A:** The per-UDF-instance memory limit — the DB's `maximal_memory_limit` — must be readable from Rust UDF code, so a downstream UDF (lakehouse-engine's DataFusion scan) can size an in-process memory pool to the limit the DB allotted.

**Q:** Does the wire protocol already carry the value, or does the proto schema need changing?
**A:** It is already carried: `exascript_info.maximal_memory_limit`, field 11, `required uint64`, units = bytes, per-UDF-instance (the DB enforces it via `setrlimit(RLIMIT_RSS)`). No proto change is needed. The gap is purely in deserialization (`UdfMeta::from_pb` drops it) and the SDK surface (`UdfContext` has no accessor).

**Q:** Which features own this, and should new features be created?
**A:** No new features. `protocol/wire-protocol` owns the handshake Info/Meta deserialization (the `conn_info` field is the closest analog). `sdk/udf-sdk` owns the `UdfContext` accessor surface. Extend both.

**Q:** What unit, and what should an absent value surface as?
**A:** Bytes, verbatim, no rescaling. The proto field is `required` so it is effectively always present; if absent on the wire, surface `0` as the sentinel for "no limit reported".

## Design Decisions

### [1] Surface the limit as raw `u64` bytes, no unit conversion

- **Decision:** `UdfMeta::maximal_memory_limit: u64` and `UdfContext::memory_limit() -> u64`, both in bytes, decoded verbatim from the proto.
- **Alternatives:** A typed `ByteSize` wrapper or `Option<u64>`; rescaling to MiB.
- **Rationale:** The proto unit is bytes and the consumer (memory-pool sizing) wants raw bytes. Matching the proto type avoids lossy conversion and an extra type, and mirrors how `node_count` is carried as a plain scalar.
- **Promotes to ADR:** no

### [2] `0` sentinel for "no limit reported" rather than `Option`

- **Decision:** Treat `0` (the prost default for the `required` field) as "no limit reported / unbounded".
- **Alternatives:** `Option<u64>` accessor; error/panic on an absent field.
- **Rationale:** The field is `required`, so prost always materialises it (defaulting to `0`). A bare `u64` keeps the common-path signature trivial; `0` is a natural unbounded sentinel and never produces a protocol error.
- **Promotes to ADR:** no

### [3] Defaulted, non-feature-gated trait accessor

- **Decision:** `memory_limit` is a provided `UdfContext` method returning `0` by default, overridden only by the host bridge; it is NOT gated behind the `connect-back` feature.
- **Alternatives:** A required trait method (forces edits to `SingleCallContext` and every test double); gating it like `cluster_ip()`.
- **Rationale:** A default keeps all existing impls compiling — the same backward-compat idiom the SDK already uses for typed getters. The limit is plain handshake metadata available unconditionally, so a connect-back gate would needlessly hide it.
- **Promotes to ADR:** no

## Review Findings

<!-- Populated by speq-implement after code review. -->
