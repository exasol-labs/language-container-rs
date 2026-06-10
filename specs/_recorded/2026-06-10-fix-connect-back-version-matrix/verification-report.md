# Verification Report: fix-connect-back-version-matrix

## Bottom Line

**PASS — connect-back via new exarrow-rs sessions works; full integration suite green on 2026.1.0.**

Connect-back (a UDF opening a new, independent SQL session back to the cluster
via exarrow-rs) now works for both **read-back** (`SELECT`) and **write-back**
(`INSERT`). The full `db_roundtrip_all_scenarios` suite passes — all 11 scenarios
including `connect_back_dml` and `connect_back_query` — in clean testcontainers
mode against `exasol/docker-db:2026.1.0`.

```
[it] scenario python3_connect_back ok
[it] scenario scalar_double ok
[it] scenario set_filter ok
[it] scenario json_parse ok
[it] scenario udf_error ok
[it] scenario single_call_default_output_columns ok
[it] scenario single_call_unimplemented ok
[it] scenario connect_back_cluster_ip ok
[it] scenario connect_back_dml ok
[it] scenario connect_back_query ok
[it] scenario connect_back_writeback_same_schema ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 13.83s
```

The `connect_back_writeback_same_schema` scenario (UDF `connect-back-crunch`)
proves the realistic write-back ordering — pre-committed target table → UDF
number-crunch + connect-back INSERT → post-UDF INSERT from a new session — and
that write-back to the **invoking query's own schema** is safe when the
Serializable-isolation rules are followed (pre-commit the target, no DDL, write
a different object than the query reads, autocommit). Verified contents of
`it_rust.crunch_log`: `(1,1) (2,4) (3,9) (4,16) (5,25)`.

---

## Root cause — four distinct bugs (none in the connect-back transport itself)

The original "connect-back is impossible / SIGABRT" conclusion was wrong. The
SIGABRT and downstream failures were **four separate, independent bugs**, all
fixed without any special Exasol API — connect-back is a plain external login.

### 1. Transaction conflict (WAIT FOR COMMIT) → SIGABRT at ~T+11 s

Exasol runs **Serializable** isolation. The connect-back session (Part:44) is a
separate transaction from the invoking query (Part:40). The original test UDF
did `CREATE TABLE` + `INSERT` + `COMMIT` **in the invoking query's own schema
(`it_rust`)**. Part:44's commit forced Part:40 into WAIT FOR COMMIT; the deadlock
detector then aborted Part:40 (`deadlock detector signalled` in the DB log →
SIGABRT).

**Fix:** the connect-back UDF writes to a **separate, pre-created** schema
(`cb_sink.cb_result`) the invoking query never locks, and performs **no DDL**.

### 2. Redundant COMMIT on an autocommit session → UDF error

exarrow-rs sessions default to autocommit; the `INSERT` commits on its own.
The explicit `conn.execute("COMMIT")` then errored (no open transaction),
surfacing as `F-UDF-CL-RUST-9001 … error code 1` *after* the data had landed.

**Fix:** removed the explicit COMMIT — rely on autocommit.

### 3. Wrong emit type for BIGINT → DB SIGSEGV in `handle_emit_request`

Exasol delivers/expects `BIGINT` as `PB_NUMERIC` (decimal string). The UDF
emitted `Value::Int64`, so the data went into the int64 block while the DB read
the empty string block → `RepeatedPtrField<string>::Get` null-deref (SIGSEGV).

**Fix:** connect-back UDFs emit `Value::Numeric` for BIGINT EMITS columns
(matching the existing `set_filter` convention).

### 4. Arrow `TypeId` mismatch across the cdylib FFI boundary → silent wrong value

`query_arrow()` returned `arrow::RecordBatch`. The UDF `.so` links its own copy
of `arrow`; `downcast_ref::<Decimal128Array>()` on a runtime-produced array
returned `None` (different `TypeId`s), so the UDF read its `unwrap_or(0)` and
emitted `0` instead of `42`.

**Fix:** added an FFI-safe `ExaConnection::query()` that converts arrow →
the SDK's own `Value` enum **inside the runtime** (where the downcast is
consistent). Only plain `Value` data crosses the FFI boundary. `query_arrow()`
is retained but documented as safe only within a single binary.

### Supporting hardening

- `build_dsn` uses the **native** protocol (no `transport=websocket`): the WS
  close handshake triggers Exasol's `safeDisconnectTimeout` (10 s) + SO_LINGER
  (1 s) before Part:44 exits; the native path drops the stream immediately,
  matching PyExasol.
- `exaudfclient` calls `std::process::exit(0)` on success so the leaked static
  connect-back Tokio runtime cannot delay process exit (which would otherwise
  let Part:40's `waitpid`/watchdog escalate to SIGABRT).

---

## Verification evidence

| Check | Result |
|-------|--------|
| `cargo fmt --check` (changed crates) | PASS |
| `cargo clippy` (exasol-udf-sdk, exa-udf-runtime, connect-back) | PASS (0 warnings) |
| `cargo test -p exasol-udf-sdk --features connect-back` | PASS (10 tests) |
| `cargo test -p exa-udf-runtime --features connect-back` | PASS |
| `db_roundtrip_all_scenarios` (2026.1.0, testcontainers) | **PASS (11/11 scenarios)** |
| connect-back INSERT data verified via exapump | `10,20,30` present in `cb_sink.cb_result` |
| connect-back SELECT round-trip | returns `42` |
| connect-back same-schema write-back | `(1,1)(2,4)(3,9)(4,16)(5,25)` in `it_rust.crunch_log` |

---

## Documentation produced

- [`docs/protocol.md`](../../../docs/protocol.md) — the Exasol UDF wire protocol (ZMQ control channel, message lifecycle, MT_IMPORT).
- [`docs/write-back-guide.md`](../../../docs/write-back-guide.md) — how to implement write-back under Serializable isolation, with a Pitfalls table.
- `docs/writing-a-udf.md` §4 corrected (cluster_ip via getifaddrs, `query()` over `query_arrow`, emit BIGINT as Numeric).

## Remaining work / follow-ups

Tracked durably in [`specs/backlog.md`](../../backlog.md) (this report is archived
on `/speq:record`, so follow-ups do not live here):

- **B-001** propagate UDF error messages (macro shim collapses `Err` → `error code 1`).
- **B-002** prefer `query()` over `query_arrow()` (arrow `TypeId` unsafe across the UDF FFI boundary).
- **B-003** richer connect-back errors carrying the failing SQL.
- **B-004** run the full suite across 2025.1.11 / 2025.2.1 / 2026.1.0 locally (CI covers it).
