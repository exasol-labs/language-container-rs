# UDF shape benchmarks

| Tier | Runs | Measures | A/B |
|---|---|---|---|
| 1 `protocol` | `cargo bench`, mock engine (`crates/exa-mock-db`), no Docker | client receive, decode, dispatch, UDF body, encode, send; every `MT_EMIT` size | Criterion `--save-baseline` / `--baseline` |
| 2 `udf-bench` | Exasol in Docker or external | the whole query including the engine; pass-through correctness | JSON per run, `compare` two sets |

Both use `benches/bench-udfs` (one cdylib, every entry point) and `benches/bench-schema`.
Column classes: `native` (`k DECIMAL(18,0), v DOUBLE`), `strblock` (`k, amount DECIMAL(18,2), d DATE, ts TIMESTAMP`),
`varchar` (`k, label VARCHAR(100)`), `wide` (24 columns, 12 nullable, emit-only: a SCALAR EMITS UDF expanding one
input row into millions of wide rows; its batch cells take rows per Arrow batch, 8192 or 65536, as third parameter).
Wire bytes per row (Tier 1 `bytes/row`): 13.9, 61.6, 57.9, 473.2.

## Tier 1

```bash
cargo bench -p exa-udf-runtime --features bench --bench protocol -- --save-baseline base   # on base
cargo bench -p exa-udf-runtime --features bench --bench protocol -- --baseline base        # on change
cargo bench -p exa-udf-runtime --features bench --bench protocol -- set_emits/native       # one group or cell
BENCH_ROWS=10000 cargo bench -p exa-udf-runtime --features bench --bench protocol -- --test  # CI smoke
```

| `BENCH_PROFILE` | rows/iteration | warm-up | measure | samples | 25 cells, 4-core Xeon 8488C |
|---|---|---|---|---|---|
| `quick` (default) | 250,000 | 1 s | 3 s | 10 | 169 s |
| `full` | 1,000,000 | 3 s | 5 s | 10 | not yet measured |

`BENCH_ROWS` overrides rows; `BENCH_ROWS_PER_CYCLE` (default 2,000) is how many SCALAR input rows the mock hands
over per `MT_RUN` cycle, calibrated on docker-db 2026.1.1 from a Tier 2 `--udf-debug` log (row count, not a byte
budget: native and varchar frames carry the same 2,000 rows at fourfold different sizes).

Groups: `scalar_returns`, `scalar_emits_gen` (incl. `wide_row`, `wide_batch8k`, `wide_batch64k`),
`scalar_emits_passthrough`, `set_returns` and `set_emits` (1 and 1,000 groups). After each group a counter table
prints `MT_EMIT` messages, rows, bytes, `bytes/row`, `max_bytes` and the count over 4,000,000 bytes, which must be 0
and is asserted, as is one `row_number` per emitted row.

## Tier 2

```bash
docker build --target artifact --output type=local,dest=/tmp/slc .   # per side, from its checkout
export SLC_TARBALL=/tmp/slc/lc-rs.tar.gz
cargo build --release -p bench-udfs
cargo run --release -p udf-bench -- run [--profile quick|full] [--rows N] [--filter set_] [--keep] [--udf-debug 172.17.0.1:5055]
cargo run --release -p udf-bench -- compare --base a.json b.json --change c.json d.json
cargo run --release -p udf-bench -- show bench-results/<commit>-<timestamp>.json
```

Docker mode boots `exasol/docker-db` (`EXASOL_VERSION`, `EXA_DB_MEM_SIZE`, default 4 GiB); external mode uses
`EXASOL_HOST`, `EXASOL_PORT`, `BUCKETFS_PORT`, `BUCKETFS_PASSWORD`. Results land in the gitignored `bench-results/`.
`--udf-debug host:port` sets `%udf_debug_level debug` and redirects the runtime log to a TCP listener on the host.

| `--profile` | rows (strblock table / wide) | warm-up | reps | 44 cells, docker-db 2026.1.1, 4 GiB | band |
|---|---|---|---|---|---|
| `quick` (default) | 250,000 (2,500 / 62,500) | 1 | 3 | 86 s incl. Docker start | ±15 % |
| `full` | 1,000,000 (10,000 / 250,000) | 1 | 5 | 264 s incl. Docker start | ±8 % |

Cells: `control_<class>`, `scalar_returns_<class>`, `scalar_emits_gen_<class>_<mode>[_noemit]`, `scalar_emits_pt`,
`set_returns_<class>_g<G>`, `set_emits_<class>_<mode>_g<G>`, `set_gen_<class>_<mode>`. Every query returns one row and
aggregates a UDF output column. `scalar_emits_pt` reports `incorrect` if an emitted row does not land beside its
input row. The DB feeds DATE/TIMESTAMP columns into a UDF at 3k to 7k rows/s on 2026.1.1 (builtin Python3 is
equally slow), so the strblock table is `n / 100` rows and those cells' `x_ctrl` is not comparable to native.
Wide cells emit `n / 4` rows; generator cells report `MB_per_s` from `WIRE_BYTES_PER_ROW` in `cells.rs`, which
must follow the Tier 1 `bytes/row` when a generator or the encoder changes.

A/B against one DB: `run` from the base checkout (A), from the change checkout (C), repeat both (B, D), then
`compare --base A B --change C D`. Mixed profiles, row counts or cells with changed rows are refused or noted.

| interval vs 0 | median delta vs band | verdict |
|---|---|---|
| excludes | outside | `improved` / `regressed` |
| excludes | inside | `small` |
| crosses | any | `no change` |

Fewer than 8 pooled samples on a side flags `low power`; Tukey outliers are flagged, never removed.

## Decisions

- `bench-udfs` is an optional dependency of the runtime behind the `bench` feature (it needs `emit-arrow`; a dev-dependency would unify that into every test build) and stays cdylib-only (UDF crates export identical symbols).
- Tier 2 compares runs, not SLCs: each side builds its own tarball and `.so`.
- Keys are `DECIMAL(18,0)` so `native` is native on the wire (BIGINT travels as a string).
- `wide` is synthetic and emit-only: no Parquet reader (that would measure the reader, not the SLC).
- CI runs the Tier 1 smoke only; `quick` is the loop, `full` is the evidence a performance PR quotes.
