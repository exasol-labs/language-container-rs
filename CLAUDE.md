# Project Rules

- Use Exasol Docker image `2026.latest` (pin this version).
- Use `exapump` to interact with Exasol.
- Do not verify SSL certificates (`validateservercertificate=0`).

## Exasol data type mapping

The DB delivers every column over the wire as one of **8 proto column types**
(`exa-proto::ColumnType`). Several SQL types collapse onto the same proto type and
are disambiguated at `ColumnMeta::from_pb` time by inspecting `type_name`. The SDK
surfaces the refined type as `exasol_udf_sdk::value::ExaType` (the single canonical
enum; `exa-zmq-protocol` re-exports it).

| Proto column type | Exasol SQL type(s) | `type_name` disambiguation | SDK `ExaType` | `Value` payload |
|-------------------|--------------------|----------------------------|---------------|-----------------|
| `PB_DOUBLE` | `DOUBLE PRECISION` (`FLOAT`, `REAL`) | none | `Double` | `Double(f64)` |
| `PB_INT32` | `DECIMAL(p,0)` small enough to fit `i32` | none | `Int32` | `Int32(i32)` |
| `PB_INT64` | `DECIMAL(p,0)` fitting `i64` | none | `Int64` | `Int64(i64)` |
| `PB_NUMERIC` | `DECIMAL(p,s)`, `BIGINT`, `NUMBER` | none | `Numeric { precision, scale }` | `Numeric(Decimal)` |
| `PB_DATE` | `DATE` | none | `Date` | `Date(NaiveDate)` |
| `PB_TIMESTAMP` | `TIMESTAMP`, `TIMESTAMP WITH LOCAL TIME ZONE` | `WITH LOCAL TIME ZONE` → `TimestampTz`, else `Timestamp` | `Timestamp` / `TimestampTz` | `Timestamp(NaiveDateTime)` / `String` (TZ) |
| `PB_STRING` | `VARCHAR`, `CHAR`, `GEOMETRY`, `HASHTYPE`, `INTERVAL YEAR TO MONTH`, `INTERVAL DAY TO SECOND` | `CHAR…` → `Char`; `VARCHAR…` → `String`; `GEOMETRY` → `Geometry`; `HASHTYPE` → `HashType`; `INTERVAL…YEAR…MONTH` → `IntervalYearToMonth`; `INTERVAL…DAY…SECOND` → `IntervalDayToSecond` | `String { size }` / `Char { size }` / `Geometry` / `HashType` / `IntervalYearToMonth` / `IntervalDayToSecond` | `String` |
| `PB_BOOLEAN` | `BOOLEAN` | none | `Boolean` | `Bool(bool)` |

Rules:
- **`BIGINT` arrives as `PB_NUMERIC`**, not `PB_INT64`. `get_i64` therefore accepts an
  integral `Value::Numeric`; it errors only on a non-zero fractional part.
- **Only ambiguous proto types consult `type_name`** (`PB_STRING`, `PB_TIMESTAMP`).
  Unambiguous types map directly and MUST NOT read `type_name`.
- **Extended types keep a `String` wire payload** (the proto block does not change);
  the `ExaType` variant — not the `Value` payload — carries the SQL distinction.
- Reference: <https://docs.exasol.com/db/latest/sql_references/data_types/datatypesoverview.htm>

## Running Exasol UDFs in CI (Ubuntu 24.04 runners)

**Rule:** before `docker run` of the Exasol DB on any Ubuntu 24.04 runner, set
`sudo sysctl -w kernel.apparmor_restrict_unprivileged_userns=0`.

Why: Exasol runs every UDF inside a sandbox built by `nschroot`, which creates an
**unprivileged user namespace** needing `CAP_SYS_ADMIN`. Ubuntu 24.04 ships
`kernel.apparmor_restrict_unprivileged_userns=1`, which strips that capability via
a restricted AppArmor profile **even under `--privileged`** → `nschroot` dies
silently → the DB reports `Internal error: VM crashed` (SQL state 22002) for
**every** UDF, built-in Python3 included. The DB itself stays healthy.

If you see "VM crashed" in CI, it is this — **not** memory, disk, the host kernel,
glibc, or the UDF code. It reproduces on Ubuntu 24.04 hosts only (local Debian has
no such restriction), so it is green locally. Confirm via `sudo dmesg` on the
runner (`apparmor="DENIED" ... comm="nschroot" capability=21 capname="sys_admin"`);
unprivileged `dmesg` is blocked and the DB-side `cored/exasql.*` logs are 0 bytes.

## Three "connection" concepts — never confuse them

| Concept | What it is | Representation in code |
|---------|------------|------------------------|
| **Exasol CONNECTION object** | A database-level credential store (`CREATE CONNECTION … TO '…' USER '…' IDENTIFIED BY '…'`). Fetched from the DB via ZMQ MT_IMPORT (`PB_IMPORT_CONNECTION_INFORMATION`). | `ConnectionObject { kind, address, user, password }` in `exasol-udf-sdk` |
| **exarrow-rs session** | A live ADBC connection/session to Exasol (or any ADBC target). Has its own transaction, completely independent of the invoking query's transaction. Opening one is the "connect-back" act. | `exarrow_rs::adbc::Connection`; exposed to UDFs as `Box<dyn ExaConnection>` |
| **Cluster node IP** | The IP of the Exasol node that started the language container. Parsed from `args[1]` (the ZMQ endpoint string `tcp://<node_ip>:<zmq_port>`). Port 8563 is the SQL endpoint. | `ctx.cluster_ip()` on `UdfContext` |

A UDF may also obtain a `ConnectionObject` for a **foreign system** (non-Exasol) and use `exarrow_rs` or any other driver directly — `connect_back` is only the Exasol-specific convenience on top.

## Connect-back: what works and what doesn't

Connect-back = the UDF opening a **new, independent SQL session** back to the cluster, exactly like any external client (PyExasol, JDBC, exarrow-rs). It is **not** an internal proxy/loopback connection.

### Serializable isolation — connect-back must not violate it

Exasol supports the **Serializable** transaction isolation level. Each transaction is carried out as if it were part of a sequence even though transactions can run in parallel. Serialization ensures data consistency but causes:

- **WAIT FOR COMMIT**: a transaction must wait for a commit from an earlier transaction before it can continue.
- **Transaction collisions** for mixed read/write transactions, which force a rollback of a transaction.

**Critical for connect-back**: the connect-back session (Part:44) runs in its **own independent transaction**, separate from the invoking query's transaction (Part:40). If the connect-back transaction modifies (INSERT/CREATE/COMMIT) an object that the invoking query's transaction also touches — or its schema — the invoking transaction enters **WAIT FOR COMMIT** and the deadlock detector eventually aborts it (observed as Part:40 SIGABRT at ~T+11 s; DB log: `deadlock detector signalled`).

**Rule: connect-back test UDFs must not violate Exasol's transaction behaviour.** A connect-back UDF that writes back must target objects/schemas the invoking query does not lock, and must not create a write-write or schema conflict with its own invoking transaction. Read-only connect-back (e.g. `SELECT 42`) is always safe; write-back requires careful object isolation.

### Connect-back is always a regular login connection

Connect-back **must** open a **regular SQL login session** using the credentials from an Exasol CONNECTION object — never a "parallel sub-connection" or any internal Exasol session type. This is the entire reason the CONNECTION object exists: it carries the `address`, `user`, and `password` needed to authenticate a new independent session exactly as any external client would.

- The CONNECTION object (`%connection MY_CONN` / `ctx.connection("MY_CONN")`) supplies the address and credentials.
- The driver (exarrow-rs, PyExasol, JDBC) opens a standard login with those credentials.
- Exasol treats the resulting session as a completely ordinary external client session — no special UDF relationship.
- Python3 `strata-rs` UDFs confirm this: `pyexasol.connect(dsn=cred.address, user=cred.user, password=cred.password, ...)` is a plain login.

**Connect-back works and is confirmed** — `strata-rs` Python UDFs create tables and insert data via connect-back against the same Docker Exasol images. The key constraint is UDF type (see below).

### UDF type rules — the most important constraint

| UDF type | Connect-back result | Reason |
|----------|--------------------|-|
| **`SET SCRIPT ... EMITS (...)`** | **Works** | Correct pattern; strata-rs Python UDFs use this exclusively |
| `SCALAR SCRIPT ... RETURNS ...` | **SIGABRT** (parent SQL process crashes ~170–250 ms after connect-back session establishes) | Exasol's SQL worker process asserts when a new session is registered while a SCALAR UDF is mid-execution; the crash is transport-agnostic (native binary and WebSocket both crash) |

**Rule: always use `SET SCRIPT ... EMITS (...)` for any UDF that does connect-back.** SCALAR UDFs must not open connect-back sessions.

The strata-rs reference pattern:
```sql
CREATE OR REPLACE PYTHON3 SET SCRIPT schema.fn(... params ...) EMITS (...) AS
%connection MY_CONN
... body ...
/

-- called as:
SELECT cols FROM (SELECT schema.fn(...) EMITS (col1 TYPE, ...) FROM DUAL)
-- or TABLE() form:
SELECT cols FROM TABLE(schema.fn(...) EMITS (col1 TYPE, ...) FROM DUAL)
```

### Address rules

| Address | Result | Reason |
|---------|--------|--------|
| `<container-eth0-ip>:8563` | **Works** (with SET SCRIPT) | Direct TCP to the node's SQL listener — same path as any external client; session is fully independent of Part:40 |
| `127.0.0.1:8563` | **SIGABRT** (~T+10 s) | Hits Exasol's internal CoreDB proxy; proxy links the new session to Part:40 so Part:40 waits for deregistration then crashes |
| Docker host gateway + mapped port | **SIGABRT** | NAT path; Exasol associates the session with the invoking session |

**Rule: always use `<container-eth0-ip>:8563` for the CONNECTION object address** — never `127.0.0.1`. Use `ctx.cluster_ip()` inside UDF code to obtain the node IP; the test harness supplies it via `container_inner_ip()` for integration tests.

### Transport rules

Both native binary (exarrow-rs default) and WebSocket (`transport=websocket`) work fine for SET SCRIPT connect-back. Transport is not the differentiating factor — UDF type is.

### ZMQ transport (UDF control channel) — separate concern

The ZMQ socket between the DB and `exaudfclient` is **IPC** (`ipc://`) on single-node Docker and **TCP** (`tcp://`) on multi-node clusters. This is chosen by the DB at launch and cannot be changed via `SCRIPT_LANGUAGES`. It has **no effect** on the connect-back SQL connection — those are always plain TCP to port 8563.

### cluster_ip()

`cluster_ip()` reads the local node's primary IPv4 via `libc::getifaddrs` (first non-loopback IPv4 = container eth0). Does **not** parse the ZMQ endpoint string. Works on both IPC (single-node Docker) and TCP (multi-node) deployments.

### exaudfclient must call std::process::exit(0) — not return normally

After the ZMQ close handshake (`MT_FINISHED` sent and received), the exaudfclient **must** call `std::process::exit(0)` — it must **not** return from `main()` normally.

**Why**: Part:40 (the Exasol SQL worker) detects exaudfclient exit via `waitpid(pid, WNOHANG)` in a polling loop (`zmqinternal.cc`). If the process lingers, the loop escalates (SIGTERM) and the `TimerWatchDog` in Part:40 eventually fires `SIGABRT` at ~T+11 s.

**Rust-specific cause**: the static `OnceLock<TokioRuntime>` for connect-back is never explicitly shut down. When `main()` returns normally, Rust's cleanup tries to join the Tokio reactor and blocking pool threads — this can delay the process exit by 10+ seconds. The reference C++ `exaudfclient_main` returns `0` immediately into `exit()`; the fix is the same: `std::process::exit(0)` at the end of `main()`.

### SLC container image (Alpine vs Debian) — spike confirmed 2026-06-09

The exaudfclient binary runs inside the **Exasol DB container's** network namespace — not inside its own container. The SLC image provides the filesystem/binary only.

**Spike results** (`test-udfs/spike-connect/`; `SELECT 42` via exarrow-rs native protocol, no transport override):

| Test context | Address | Result |
|---|---|---|
| Host → DB | `172.17.0.2:8563` | ✓ |
| Inside DB container (`docker exec`) | `172.17.0.2:8563` | ✓ |
| Inside Alpine SLC container | `172.17.0.2:8563` | ✓ |
| Inside Debian SLC container | `172.17.0.2:8563` | ✓ |
| Inside freshly rebuilt Alpine SLC | `172.17.0.2:8563` | ✓ |

**Conclusion:** exarrow-rs native protocol connects successfully to `<container-eth0-ip>:8563` from every relevant execution context. Alpine vs Debian makes no difference.
