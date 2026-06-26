# Emit-throughput benchmark — Rust SLC vs native Python3

Verifies the Rust SLC's emit path is as fast as it can be, with the builtin
Python3 SLC as the baseline. Each run is decomposed into three measure points:

1. **SLC startup** — cold first call (process spawn + `dlopen` / interpreter import).
2. **Data generation** — build all N rows, emit one sentinel (`do_emit=0`).
3. **Data transfer** — emit + fetch all N rows (`do_emit=1`); `T_transfer = T_full − T_generation`.

**Data transfer (rows/s, MB/s) is the headline metric.** Matrix: mixed shape
(`id BIGINT, label VARCHAR(100), val DOUBLE`) × {row, columnar} × {Rust, Python3}
× {1M, 5M}. Median of 5 runs.

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
