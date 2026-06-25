# Verification Report: add-emit-batch-arrow

## Verdict: ✅ PASS

`ctx.emit_batch(&RecordBatch)` ships behind a new `emit-arrow` SDK feature. The
final design serialises the batch to **Arrow IPC bytes inside the UDF `.so`** and
crosses the cdylib boundary only as `&[u8]`; the host deserialises into its own
`RecordBatch` and encodes it column-at-a-time into the proto type blocks via the
existing `EmitBuffer::push_batch`. All unit, lint, format, and the live-DB
end-to-end suite are green, including the new `emit_arrow_batch` round-trip.

A mid-implementation design pivot is recorded: the original plan passed
`&RecordBatch` across the boundary directly; the live-DB E2E proved that SIGSEGVs
(two independently linked static `arrow` copies disagree on `Arc<dyn Array>`
vtables / `TypeId` — the hazard documented in CLAUDE.md and backlog B-002). The
IPC-bytes redesign keeps Arrow off the boundary. See `decision-log.md`
("E2E Outcome") and backlog **B-005** (raw-buffer optimisation, deferred).

## Automated Checks

| Step | Command | Result |
|------|---------|--------|
| Format | `cargo fmt --check` | ✅ clean |
| Lint | `cargo clippy --all-targets --all-features -- -D warnings` | ✅ no issues |
| Build | `cargo build --release` | ✅ exit 0 |
| SDK feature build | `cargo build -p exasol-udf-sdk --features emit-arrow` / `--no-default-features` | ✅ arrow present with feature, absent without |
| Runtime feature build | `cargo build -p exa-udf-runtime --features emit-arrow` / `--features connect-back` | ✅ exit 0 (connect-back implies emit-arrow) |
| Unit — SDK | `cargo test -p exasol-udf-sdk --features emit-arrow` | ✅ 11 passed |
| Unit — runtime | `cargo test -p exa-udf-runtime --features emit-arrow` | ✅ push_batch + bridge tests pass |
| Boundary gate (no DB) | `cargo test -p exa-udf-runtime --features emit-arrow --test emit_arrow_dlopen` | ✅ `.so`-built batch round-trips via IPC (previously SIGSEGV'd) |
| Integration / E2E | `scripts/ci-it-local.sh` (live Exasol 2026.1.0, freshly built SLC) | ✅ all scenarios; `Done (rc=0)` |

> Note: `cargo test` (default, no features) shows 2 pre-existing `dispatch.rs`
> failures that only require the fixture `.so`s to be rebuilt after the version
> bump (stale `sdk_fingerprint`); they pass once `scalar-double`,
> `annotated-fixture`, `single-call-fixture` are built. Not related to this change.

## Scenario Coverage

| Scenario | Test | Result |
|----------|------|--------|
| `emit-arrow` pulls in arrow independently of connect-back | SDK Cargo feature build matrix | ✅ |
| `emit_batch` serialises to IPC; Arrow never crosses the `.so` boundary | `default_emit_batch_unimplemented` + `emit_arrow_dlopen` boundary gate | ✅ |
| `EmitBuffer` encodes a batch column-at-a-time (one downcast/null-read per column) | `push_batch_equals_row_push`, `push_batch_shared_block_type_interleaved`, `push_batch_null_bitmap`, `push_batch_int64_into_numeric_block` | ✅ |
| `push_batch` splits an oversized batch at row boundaries under the 4 MB cap | `push_batch_splits_oversized_batch`, `push_batch_slice_zero_copy_tail_bounded` | ✅ |
| `push_batch` is byte-identical to the row-based push path | `push_batch_equals_row_push` | ✅ |
| Bridge deserialises IPC, carries output metadata, flushes on the same threshold | `bridge_emit_batch_buffers_and_flushes`, `bridge_emit_batch_error_propagates` | ✅ |
| `emit` and `emit_batch` share one buffer and one tail flush | `bridge_mixed_emit_styles_share_buffer` | ✅ |
| A column whose Arrow type cannot feed the declared `ExaType` errors | `push_batch_type_mismatch_errors` | ✅ |
| `emit-arrow-batch` fixture emits a manually built RecordBatch end-to-end | `emit_arrow_batch_roundtrips` (live DB) | ✅ |

## Manual Testing

| Feature | Command | Result |
|---------|---------|--------|
| sdk/udf-sdk | `cargo build -p exasol-udf-sdk --features emit-arrow` then `--no-default-features` | ✅ first links arrow, second has no arrow dep |
| runtime | `cargo test -p exa-udf-runtime --features emit-arrow push_batch` | ✅ pass |
| examples/test-udfs (e2e) | `scripts/ci-it-local.sh` → `[it] scenario emit_arrow_batch ok` | ✅ emitted rows equal the UDF-built batch (1:a, 2:b, 3:c) |

## Version

Workspace bumped `0.16.0` → `0.17.0` (additive, opt-in feature); `Cargo.lock` in sync.
