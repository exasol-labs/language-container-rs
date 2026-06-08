# Project Rules

- Use Exasol Docker image `2026.latest` (pin this version).
- Use `exapump` to interact with Exasol.
- Do not verify SSL certificates (`validateservercertificate=0`).

## Three "connection" concepts — never confuse them

| Concept | What it is | Representation in code |
|---------|------------|------------------------|
| **Exasol CONNECTION object** | A database-level credential store (`CREATE CONNECTION … TO '…' USER '…' IDENTIFIED BY '…'`). Fetched from the DB via ZMQ MT_IMPORT (`PB_IMPORT_CONNECTION_INFORMATION`). | `ConnectionObject { kind, address, user, password }` in `exasol-udf-sdk` |
| **exarrow-rs session** | A live ADBC connection/session to Exasol (or any ADBC target). Has its own transaction, completely independent of the invoking query's transaction. Opening one is the "connect-back" act. | `exarrow_rs::adbc::Connection`; exposed to UDFs as `Box<dyn ExaConnection>` |
| **Cluster node IP** | The IP of the Exasol node that started the language container. Parsed from `args[1]` (the ZMQ endpoint string `tcp://<node_ip>:<zmq_port>`). Port 8563 is the SQL endpoint. | `ctx.cluster_ip()` on `UdfContext` |

A UDF may also obtain a `ConnectionObject` for a **foreign system** (non-Exasol) and use `exarrow_rs` or any other driver directly — `connect_back` is only the Exasol-specific convenience on top.
