# Decision Log: add-connect-back-batch-execute

Date: 2026-06-19

## Interview

**Q: Build the batch wire API or consume it?**
**A:** Consume only. exarrow-rs 0.12.8 already ships `prepare` (src/adbc/connection.rs:378), `execute_batch_update` (522), `close_prepared` (626), `PreparedStatement::build_batch_parameters_data` (src/query/prepared.rs:113), and the `Parameter` enum (src/query/statement.rs:69). No exarrow-rs change. lc-rs only bumps the pin and adds the trait surface + runtime impl.

**Q: Binary parameters — add a `Value::Binary` variant now, or rely on hex-encoded `Value::String`?**
**A:** Rely on hex-`String` for this release to avoid touching the `Value` enum, provided Exasol's prepared-statement protocol accepts a bound `VARCHAR` parameter where a byte-typed column cast expects it (e.g. `CAST(? AS HASHTYPE(n BYTE))`). If a future client hits a column type that rejects this, fall back to adding `Value::Binary(Vec<u8>)` (+ matching `ExaType` + serialization in sdk/udf-sdk) mapped to `Parameter::Binary`. Keep the change minimal: add the variant only when a concrete need forces it.

**Q: Scope/laziness?**
**A:** Ponytail — minimal. One trait method + one runtime impl + dep/version bump. Only add `Value::Binary` if a concrete client need forces it.

## Design Decisions

### [1] No `Value::Binary` in this release — binary values bind as hex `Value::String`

- **Decision:** Do NOT add a `Value::Binary` variant. Clients that must bind byte-oriented column types (e.g. an Exasol `HASHTYPE`, bound via `CAST(? AS HASHTYPE(n BYTE))`) hex-encode their bytes into a `Value::String`.
- **Validation basis:** Inspection of exarrow-rs `parameter_to_json` in `src/query/prepared.rs:173-181`: `Parameter::String(s)` serialises to `serde_json::Value::String(s)` and `Parameter::Binary(b)` serialises to `serde_json::Value::String(hex::encode(b))`. Both produce identical JSON on the wire — a hex string. The Exasol prepared-statement protocol represents column values as JSON; byte-typed columns such as HASHTYPE accept hex-string literals (Exasol docs: "HASHTYPE values are stored as hexadecimal strings"), so the prepared-statement executor performs the same implicit cast as `CAST('<hex>' AS HASHTYPE(n BYTE))`. Therefore `Parameter::String(hex)` and a hypothetical `Parameter::Binary(raw)` produce the same wire bytes for an even-length hex string — adding the variant would buy nothing on the wire.
- **Risk flag / upgrade path:** This rests on protocol inspection and documentation, not a live smoke test (`⚠ smoke-test-before-finalise` if a live test is desired). If a future client hits a column type that rejects a hex-string parameter at runtime, add `Value::Binary(Vec<u8>)` to `Value`, map it to `Parameter::Binary`, add `ExaType::Binary`, and update the `sdk/udf-sdk` spec; that client then passes raw bytes instead of hex.
- **Caller consequence:** A client binding binary data MUST hex-encode it before passing it as `Value::String` to `execute_batch`.
- **Promotes to ADR:** no

### [2] execute_batch returns Unimplemented by default on the trait

- **Decision:** `fn execute_batch` has a default body returning `Err(UdfError::Unimplemented(...))`, mirroring the `begin`/`commit`/`rollback` pattern (crates/exasol-udf-sdk/src/connect_back.rs:90-114).
- **Alternatives:** Make `execute_batch` a required method. Rejected — all existing mock implementations in unit tests would fail to compile without implementing a method they do not need.
- **Rationale:** A default-Unimplemented method is the established pattern in this codebase for extending the trait without breaking implementors. The trait comment "connections that do not manage transactions (e.g. test mocks) continue to compile" already establishes the pattern.
- **Promotes to ADR:** no

### [3] Numeric/Date/Timestamp parameters return Unimplemented rather than string-rendering

- **Decision:** `value_to_parameter` returns `Err(UdfError::Unimplemented(...))` for `Value::Numeric`, `Value::Date`, and `Value::Timestamp`. The mapped variants (String, Integer from Int32/Int64, Float, Boolean, Null) cover the common batch-DML cases; the others are not yet needed.
- **Alternatives:** Silently render unsupported variants to their string form (e.g. `Numeric` → `"123.45"`). Rejected — a silently wrong literal can cause data corruption without a clear error message. For example, `Value::Numeric { unscaled: 12345, scale: 2 }` rendered as `"12345"` would insert the wrong value into a numeric column.
- **Rationale:** An explicit error is actionable; a wrong value is not. Callers that genuinely need Numeric/Date/Timestamp can convert to `Value::String` before calling `execute_batch`.
- **Promotes to ADR:** no

### [4] close_prepared errors are logged but do not replace the execution error

- **Decision:** If `Connection::execute_batch_update` succeeds but `Connection::close_prepared` fails, the success value is returned and the close error is logged. If `execute_batch_update` fails, `close_prepared` is still attempted (resource cleanup) but its error does not replace the original error.
- **Alternatives:** Surface the close error. Rejected — the close error (a resource-leak concern) is less actionable than the original execution error (a data-integrity concern), and replacing the error would hide the root cause.
- **Rationale:** Same pattern used by most client libraries (JDBC `PreparedStatement.close()` in a finally block: errors discarded). A leak of a server-side statement handle is tolerable; an invisible execution failure is not.
- **Promotes to ADR:** no

### [5] Empty rows fast-path returns Ok(0) without a server round-trip

- **Decision:** If `rows.is_empty()`, `execute_batch` returns `Ok(0)` immediately without calling `prepare`, `execute_batch_update`, or `close_prepared`.
- **Alternatives:** Allow the prepared-statement path to handle an empty slice (exarrow-rs `build_batch_parameters_data` returns `Ok(None)` for empty rows). Not adopted — avoiding a prepare/close round-trip for a no-op delete is cheaper and semantically correct.
- **Promotes to ADR:** no

### [6] Version bumped to 0.13.0 (MINOR) for the new trait method

- **Decision:** Bump `[workspace.package].version` from `0.12.1` to `0.13.0`. The addition of `execute_batch` to `ExaConnection` is a new public method — a MINOR change per semver.
- **Rationale:** The default implementation means existing code compiles unchanged, so this is not a breaking change. MINOR correctly signals new capability.
- **Promotes to ADR:** no

## Review Findings

<!-- Populated by speq-implement after code review. -->
