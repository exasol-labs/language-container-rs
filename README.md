# slc-rs — Rust Script Language Container for Exasol

A slim Exasol language container that executes precompiled Rust UDFs (`.so`
artifacts uploaded to BucketFS) via the native ZMQ+Protobuf SLC protocol.

## Connect-Back

Connect-back lets a UDF query the database from inside its `run()` call.

### Operator setup

Create a named `CONNECTION` object pointing at a routable cluster endpoint:

```sql
CREATE CONNECTION CB_SELF
  TO 'your-cluster-host:8563'
  USER 'sys'
  IDENTIFIED BY 'exasol';
```

### UDF usage

Reference the connection by name in the `%connection` directive; the runtime
resolves credentials at execution time — the artifact hardcodes nothing:

```sql
CREATE OR REPLACE RUST SCALAR SCRIPT my_udf() RETURNS BIGINT AS
%connection CB_SELF
%udf_object /buckets/bfsdefault/default/udf/my_udf.so;
/
```

### Semantics

Connect-back always opens a **new external-client session** and a **new
transaction**. The Exasol core cannot share the invoking query's transaction
with a language-container UDF.

### Known issue — `2026.latest` server-side SIGABRT

On `exasol/docker-db:2026.latest` (image id `b81d80f63d10`, same as `2026.1.0`)
the server kills the invoking session with `SIGABRT` (signal 6) the moment the
UDF opens a connect-back connection. The crash is **server-side** and happens
regardless of the connect-back address or transport. The SLC implementation is
correct and matches the Python/Java reference containers; the blocker is an
upstream Exasol core defect (see ADR-015).

The two connect-back integration scenarios are retained in the test suite as
**known-failing gates** — they will turn green automatically once Exasol ships
a patched image.
