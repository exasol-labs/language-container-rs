# Emit-throughput benchmark — Rust SLC vs native Python3

Verifies the Rust SLC's emit path is as fast as it can be, with the builtin
Python3 SLC as the baseline. Each run is decomposed into four measure points:

1. **SLC startup** — cold first call (process spawn + `dlopen` / interpreter import).
2. **Data generation** — build all N rows, emit one sentinel (`do_emit=0`).
3. **Data transfer (emit)** — emit + fetch all N rows (`do_emit=1`);
   `T_transfer = T_full − T_generation`.
4. **Data transfer (ingest, Rust only)** — decode-side cost of
   `InputRowSet::from_proto` / `decode_string_block`: a `sink_<shape>` SET
   script reads every column of every row produced by `emit_<shape>_<mode>`
   and reports the row count back; `T_ingest = T_ingest_full − T_full` (the
   emit `T_full` already measured in point 3, for the same shape/mode/N).

**Data transfer (rows/s, MB/s) is the headline metric.** Matrix: two shapes ×
{row, columnar} × {Rust, Python3} × {1M, 5M}. Median of 5 runs.

## Shapes

- **mixed** — `id BIGINT, label VARCHAR(100), val DOUBLE` (~66 B/row). No
  NUMERIC/DATE/TIMESTAMP string-block columns.
- **wide** — `id BIGINT, amount DECIMAL(18,2), event_date DATE, event_ts
  TIMESTAMP, label VARCHAR(100)` (~106 B/row). Exercises
  `value_to_block_string`'s `chrono`- and `Decimal`-`Display`-based formatting
  for all three string-block temporal/numeric types on the emit side, and the
  mirror `decode_string_block` parsing on the ingest side — added because the
  original `mixed`-only benchmark had zero coverage of the types under
  suspicion of dominating `emit_batch`'s cost (see
  `specs/_plans/add-emit-transfer-spikes/plan.md`).

Both shapes have Rust (`emit_<shape>_row`, `emit_<shape>_batch`,
`sink_<shape>`) and Python3 (`py_<shape>_row`, `py_<shape>_batch`) scripts;
ingest is measured for the Rust runtime only (`InputRowSet::from_proto` /
`decode_string_block` are Rust-runtime-internal, not exercised by Python3's
own decode path).

## Run (Docker, Exasol 2026.1, EXA_DB_MEM_SIZE=4 GiB)

```bash
docker build -f Dockerfile.alpine --target artifact --output type=local,dest=/tmp/slc .
export SLC_TARBALL=/tmp/slc/lc-rs.tar.gz
cargo build --release -p emit-bench-udf      # the bench UDF .so
cargo run  --release -p emit-bench           # boots the DB and prints the table
```

## Run against an external Exasol

Set the harness external-mode vars, then run the bench (it uses `exapump` to wait
for readiness):

```bash
export EXASOL_HOST=... EXASOL_PORT=8563 BUCKETFS_PORT=2581 BUCKETFS_PASSWORD=...
export SLC_TARBALL=/tmp/slc/lc-rs.tar.gz
cargo run --release -p emit-bench
```

Start that DB with `EXA_DB_MEM_SIZE='4 GiB'`. Override the Docker image via
`EXASOL_VERSION` / `EXASOL_DB_SERIES`; override DB memory via `EXA_DB_MEM_SIZE`.

> The Python3 columnar cells show `N/A` if pandas is absent from the builtin Python3.
