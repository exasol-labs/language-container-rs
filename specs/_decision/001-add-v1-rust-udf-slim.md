# Decisions: add-v1-rust-udf-slim

## ADR: v1 uses Option A (precompiled .so) only; JIT returns unsupported

**ID:** v1-option-a-precompiled-so-only
**Plan:** `add-v1-rust-udf-slim`
**Status:** Accepted

### Context

JIT compilation (Option C) needs a ~1.4 GB image with a vendored Cargo registry and an in-container compile/cache pipeline. Precompiled `.so` loading (Option A) covers the full protocol, SDK, loader and dispatch surface without that infrastructure.

### Decision

The runtime executes only precompiled `.so` artifacts (Option A). A script without a `%udf_object` directive, which would need the JIT path (Option C), fails with an unsupported-feature error.

### Options Considered

| Option | Verdict |
|--------|---------|
| Option A only (precompiled .so) | ✓ Chosen, least infrastructure |
| Option C (JIT) also | ✗ Needs the ~1.4 GB image, vendored registry and in-container pipeline |

### Consequences

The slim image supports only `.so` artifacts uploaded to BucketFS.

## ADR: Pure I/O-free protocol state machine separated from ZMQ transport

**ID:** pure-io-free-protocol-state-machine
**Plan:** `add-v1-rust-udf-slim`
**Status:** Accepted

### Context

The protocol state machine handles more than a dozen message types and phase transitions. Socket I/O inside it would make it untestable without libzmq.

### Decision

`exa-zmq-protocol::Protocol` consumes decoded `ExascriptResponse` values and produces `ExascriptRequest`/`HostEvent` values with no socket I/O. The REQ socket lives only in `ZmqTransport`.

### Options Considered

| Option | Verdict |
|--------|---------|
| Pure state machine, I/O in ZmqTransport | ✓ Chosen, unit-testable with fixtures |
| Socket I/O inside the state machine | ✗ Message-ordering logic not deterministically testable |

### Consequences

Only `ZmqTransport` needs integration-level tests.

## ADR: Single C-ABI crossing with ABI-version and fingerprint gating

**ID:** single-c-abi-crossing-abi-version-fingerprint
**Plan:** `add-v1-rust-udf-slim`
**Status:** Accepted

### Context

Rust has no stable ABI, so rich trait objects across a `dlopen` boundary risk undefined behavior. The loader must reject toolchain mismatches before they cause it.

### Decision

Each UDF crosses the FFI boundary through one `extern "C"` entry point per script name that returns a `#[repr(C)]` vtable. The loader checks the vtable's ABI version and SDK fingerprint before it calls any slot. The `#[exasol_udf]` macro embeds the build-time fingerprint and catches panics in every slot it generates.

### Options Considered

| Option | Verdict |
|--------|---------|
| Single `#[repr(C)]` vtable with abi_version and fingerprint | ✓ Chosen |
| Rich trait objects across the boundary | ✗ No stable ABI, undefined behavior |
| No fingerprint check | ✗ Toolchain mismatch becomes silent undefined behavior |

### Consequences

A `.so` built with a mismatched toolchain or SDK is rejected at load time. A panic in UDF code becomes an error code instead of unwinding across FFI. Any change to the vtable layout, or to the signature of a method reached through it, bumps `EXA_UDF_ABI_VERSION`, so the loader rejects `.so` files built against an older layout and they must be recompiled.
