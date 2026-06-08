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
use exasol_udf_sdk::context::UdfContext;
use exasol_udf_sdk::error::UdfError;
use exasol_udf_sdk::value::Value;

#[exasol_udf]
pub fn scalar_double(ctx: &mut dyn UdfContext) -> Result<(), UdfError> {
    let doubled = match ctx.get(0)? {
        Value::Int64(n)   => Value::Int64(n * 2),
        Value::Numeric(s) => {
            let n: i64 = s.parse().map_err(|e| UdfError::Type(format!("cannot parse '{}': {}", s, e)))?;
            Value::Numeric((n * 2).to_string())
        }
        Value::Null => Value::Null,
        _           => return Err(UdfError::Type("expected Int64 or Numeric".into())),
    };
    ctx.emit(&[doubled])
}
```

Build with `cargo exaudf build` (musl, release), upload the `.so` to BucketFS, then:

```sql
CREATE OR REPLACE RUST SCALAR SCRIPT my_schema.double(val BIGINT) RETURNS BIGINT AS
%udf_object /buckets/bfsdefault/default/udf/libscalar_double.so;
/
```

## Connect-back

UDFs can open a live Exasol session from inside `run()` to query or write back to the database. See [docs/writing-a-udf.md](docs/writing-a-udf.md#4-connect-back) for the full pattern.

## Crates

| Crate | Audience | Purpose |
|-------|----------|---------|
| `exasol-udf-sdk` | UDF authors | Trait, macros, types |
| `exa-udf-runtime` | Container operators | ZMQ host-dispatch runtime |
| `cargo-exaudf` | Build tooling | Build/validate `.so` UDF artifacts |

See [docs/cargo-ecosystem.md](docs/cargo-ecosystem.md) for the full workspace layout.

## License

MIT © Exasol AG — see [LICENSE](LICENSE).
