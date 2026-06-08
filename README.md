# language-container-rs

![Rust 1.84+](https://img.shields.io/badge/rust-1.84%2B-orange.svg)
![Status: Alpha](https://img.shields.io/badge/status-alpha-yellow.svg)
![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)
![spec|driven](https://img.shields.io/badge/spec-driven-blueviolet.svg)
![Exasol|database](https://img.shields.io/badge/Exasol-database-brightgreen.svg)

A pure-Rust Exasol Script Language Container that executes precompiled `.so` UDFs from BucketFS via the native ZMQ+Protobuf SLC protocol.

## What it is

`language-container-rs` replaces the C++ launcher and `libexaudflib` entirely. UDFs are compiled to `x86_64-unknown-linux-musl` shared libraries, uploaded to BucketFS, and loaded at runtime through a thin FFI dispatch loop. No C++ toolchain, no JVM, no Python runtime.

The workspace ships three crates for UDF authors, container operators, and build tooling — plus the protocol layer that wires them together.

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
    let out = match ctx.get(0)? {
        Value::Int64(n) => Value::Int64(n * 2),
        Value::Null     => Value::Null,
        _               => return Err(UdfError::Type("expected BIGINT".into())),
    };
    ctx.emit(&[out])
}
```

**Build**

```bash
cargo exaudf build
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

| Crate | Audience | Purpose |
|-------|----------|---------|
| `exasol-udf-sdk` | UDF authors | Trait, macros, types |
| `exa-udf-runtime` | Container operators | ZMQ host-dispatch runtime |
| `cargo-exaudf` | Build tooling | Build/validate `.so` UDF artifacts |

## Documentation

| | |
|---|---|
| [Writing a Rust UDF](docs/writing-a-udf.md) | Implement, test, build and deploy a UDF from scratch |
| [Connect-back](docs/writing-a-udf.md#4-connect-back) | Query or write to the database from inside a UDF |
| [Cargo ecosystem](docs/cargo-ecosystem.md) | Workspace layout, feature flags, build tooling |

Full index → [docs/index.md](docs/index.md)

---
MIT © Exasol AG
