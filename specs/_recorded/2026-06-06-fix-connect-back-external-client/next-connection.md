# Next: Auto-Discover Cluster IP from ZMQ Endpoint

## Finding

The Exasol node IP can be extracted programmatically from the ZMQ endpoint
that the DB passes to `exaudfclient` as `args[1]`:

    tcp://<node_ip>:<zmq_port>

Parsing `<node_ip>` and appending `:8563` yields the SQL endpoint the UDF
can use for connect-back — without any operator-configured `CREATE CONNECTION`.

## Code Path

1. `crates/exaudfclient/src/main.rs` — `endpoint = &args[1]`
2. `crates/exa-udf-runtime/src/lib.rs` — `Runtime { endpoint: String, … }`
   The endpoint is already stored here; not yet threaded through to connect_back.
3. `crates/exa-udf-runtime/src/connect_back.rs` — `build_dsn` currently takes
   the address from `PB_IMPORT_CONNECTION_INFORMATION` (MT_IMPORT). A future path
   would parse `Runtime.endpoint` as a fallback when `%connection` is absent.

## Implications for the Next Plan

- `%connection NAME` directive can become **optional** for connect-back.
  - If present: use the named connection's address (current behaviour, portable).
  - If absent: parse `Runtime.endpoint` → `<node_ip>:8563` (auto-discovery).
- UDF authors writing single-cluster scripts would no longer need an operator to
  create and maintain a `CREATE CONNECTION` object.

## What This Doesn't Solve

- **ADR-015 (SIGABRT)** — the SIGABRT is server-side and address-independent.
  The auto-discovered IP is the executing node's eth0; the crash fires regardless
  of whether the UDF connects to the container eth0, Docker host gateway, or any
  other routable address. Resolution awaits an Exasol server patch.
- **Multi-node clusters** — `<node_ip>` is the *executing* node's IP only.
  For clusters behind a VIP or load balancer, the UDF still connects to one
  specific node's SQL port. This is correct (the UDF runs on that node), but
  the result set is node-scoped unless the SQL is globally routable on that node.

## MT_IMPORT Default Behaviour (Unverified)

When `PB_IMPORT_CONNECTION_INFORMATION` is sent without a named connection,
the DB likely returns a loopback address (`127.0.0.1:8563` or similar CoreDB
address). This has not been verified empirically. Probing this is a prerequisite
for any plan that makes `%connection` fully optional.
