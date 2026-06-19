<div align="center">

![language-container-rs logo](assets/logo.svg)

![Rust 1.92+](https://img.shields.io/badge/rust-1.92%2B-orange.svg)
![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)
![spec|driven](https://img.shields.io/badge/spec-driven-blueviolet.svg)
![Exasol|database](https://img.shields.io/badge/Exasol-database-brightgreen.svg)

A pure-Rust Exasol Language Container that executes precompiled `.so` UDFs from BucketFS.

</div>

## What it is

[Exasol](https://www.exasol.com) is a high-performance analytic database built for speed and scalability. You can try it immediately with [Exasol Personal](https://github.com/exasol/exasol-personal), the [SaaS free trial](https://cloud.exasol.com) or spin up a local [Docker image](https://hub.docker.com/r/exasol/docker-db).

`language-container-rs` is the Rust Language Container for Exasol. It lets data engineers write UDFs in Rust — compiled to `.so` shared libraries, uploaded to BucketFS once, and loaded at query time. Third-party crates are statically linked into the `.so`, so adding a dependency never requires redeploying the language container.

The workspace ships three crates for UDF authors, container operators, and build tooling — plus the protocol layer that wires them together.

## Prerequisites

- **Docker** — to build the language container image
- **[exapump](https://github.com/exasol-labs/exapump)** — to upload to BucketFS and run SQL
- **Rust 1.92+** with `cargo` — to compile UDFs
- An Exasol instance: [Exasol Personal](https://github.com/exasol/exasol-personal), [SaaS free trial](https://cloud.exasol.com), or [Docker image](https://hub.docker.com/r/exasol/docker-db)

## Install the language container

`scripts/install.sh` builds the Docker image, uploads it to BucketFS, and registers the `RUST` script language in one command. See [Installation](docs/installation.md) for the full walkthrough, including how to read the BucketFS write password.

## Quick start

```rust
use exasol_udf_macros::exasol_udf;
use exasol_udf_sdk::{
    context::UdfContext,
    error::UdfError,
    value::Value,
};

#[exasol_udf]
pub fn double(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    // get_i64 transparently accepts BIGINT, which Exasol delivers as
    // Value::Numeric on the wire — no manual variant matching needed.
    let out = match ctx.get_i64(0)? {
        Some(n) => Value::Int64(n * 2),
        None    => Value::Null,
    };
    ctx.emit(&[out])
}
```

**Build**

```bash
cargo exasol-udf build
# → target/x86_64-unknown-linux-musl/release/libdouble.so
```

**Deploy**

```bash
exapump bfs upload \
    target/x86_64-unknown-linux-musl/release/libdouble.so \
    /buckets/bfsdefault/default/udf/libdouble.so
```

**Create script**

```sql
CREATE OR REPLACE RUST SCALAR SCRIPT my_schema.double(val BIGINT)
RETURNS BIGINT AS
%udf_object /buckets/bfsdefault/default/udf/libdouble.so;
/
```

## Crates

Three crates are published to [crates.io](https://crates.io) for UDF authors; the protocol and runtime crates are internal to the container.

| Crate | Audience | Purpose |
|-------|----------|---------|
| [`exasol-udf-sdk`](https://crates.io/crates/exasol-udf-sdk) | UDF authors | `UdfRun`/`UdfContext` traits, `Value`/`ExaType`, Arrow bridge |
| [`exasol-udf-macros`](https://crates.io/crates/exasol-udf-macros) | UDF authors | `#[exasol_udf]` proc-macro |
| [`cargo-exasol-udf`](https://crates.io/crates/cargo-exasol-udf) | Build tooling | Scaffold, build (static musl `.so`), validate |

## Documentation

| | |
|---|---|
| [Installation](docs/installation.md) | Build, upload and register the container; read the BucketFS write password |
| [Writing a Rust UDF](docs/writing-a-udf.md) | Implement, test, build and deploy a UDF from scratch |
| [Exasol UDF protocol](docs/protocol.md) | The ZMQ REQ/REP + Protobuf SLC wire protocol |
| [Cargo ecosystem](docs/cargo-ecosystem.md) | Workspace layout, feature flags, build tooling |

Full index → [docs/index.md](docs/index.md)

## License

Community-supported. Licensed under [MIT](LICENSE).

---
<div align="center">Built with Rust 🦀 and made with ❤️ as part of <a href="https://github.com/exasol-labs">Exasol Labs</a> 🧪.</div>
