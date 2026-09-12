# UDF shape benchmarks

Two tiers, one bench UDF (`benches/bench-udfs`), one shared column schema
(`benches/bench-schema`). Base against change: Tier 1
resolves client-side deltas of a few percent in seconds; Tier 2 shows whether
they survive the database.

| Tier | Where | Sees | Base vs change |
|------|-------|------|----------------|
| 1 `protocol` | `cargo bench`, mock engine, no Docker | receive, decode, dispatch, UDF body, encode, send; every `MT_EMIT` size | Criterion `--save-baseline` / `--baseline` |
| 2 `udf-bench` | Exasol in Docker or external | the whole query including the engine; pass-through correctness | one JSON per run, `compare` two sets |

Column classes: `native` (`k DECIMAL(18,0), v DOUBLE`), `strblock`
(`k, amount DECIMAL(18,2), d DATE, ts TIMESTAMP`), `varchar` (`k, label VARCHAR(100)`), and
`wide`: 24 columns (8 native incl. BOOLEAN, 8 string-block incl. `DECIMAL(36,10)`, 8 VARCHAR
of 8 to 200 characters with lengths varying per row), 12 of them nullable with one NULL in
ten rows. `wide` is emit-only and models a SCALAR EMITS UDF that expands one input row (a
file reference) into millions of wide rows; its batch cells take the rows per Arrow record
batch (`8192`, `65536`) as the script's third parameter, the knob such a UDF controls.
Measured wire size per row (Tier 1 `bytes/row`): native 12.9, strblock 60.6, varchar 56.9,
wide 472.2 bytes.

## Tier 1: protocol benchmark

```bash
cargo bench -p exa-udf-runtime --features bench --bench protocol -- --save-baseline base   # on base
cargo bench -p exa-udf-runtime --features bench --bench protocol -- --baseline base        # on change
cargo bench -p exa-udf-runtime --features bench --bench protocol -- set_emits/native       # one group or cell
BENCH_ROWS=10000 cargo bench -p exa-udf-runtime --features bench --bench protocol -- --test  # CI smoke
```

| `BENCH_PROFILE` | rows/iteration | warm-up | measure | samples | 25 cells (4-core Xeon 8488C, 30 GB) |
|---|---|---|---|---|---|
| `quick` (default) | 250,000 | 1 s | 3 s | 10 | about 3 min (169 s) |
| `full` | 1,000,000 | 3 s | 5 s | 10 | not yet measured |

`BENCH_ROWS` overrides the row count; `BENCH_ROWS_PER_CYCLE` sets how many SCALAR
input rows the mock hands over per `MT_RUN` cycle (default 2,000, calibrated on
docker-db 2026.1.1 from a Tier 2 debug log: `native` and `varchar` input frames carry the
same 2,000 rows while their byte sizes differ fourfold, so the cycle is a row count, not a
byte budget; a query spreads over several UDF processes, each receiving its own frames).

Groups: `scalar_returns` (3 classes), `scalar_emits_gen` (3 classes × row/batch, plus
`wide_row`, `wide_batch8k`, `wide_batch64k`), `scalar_emits_passthrough` (native),
`set_returns` (native, strblock × 1 and 1,000 groups), `set_emits` (native, strblock ×
row/batch × 1 and 1,000 groups).

```
set_emits/native_batch_g1   time:   [56.9 ms 57.7 ms 58.5 ms]     # window: MT_RUN reply → client MT_DONE
                            thrpt:  [4.27 Melem/s 4.33 Melem/s 4.39 Melem/s]
                            change: [-3.1% -1.2% +0.8%] (p = 0.21 > 0.05)   # vs --baseline; p > 0.05 is noise
[set_emits] MT_EMIT per iteration
cell                 msgs   rows    bytes   bytes/row  mean_bytes  max_bytes  >4000000  row_number
native_batch_g1000 1000.0  250000  139846   0.6        140         140        0.0       no   # >4000000 must stay 0
```

## Tier 2: end to end on Exasol

```bash
docker build --target artifact --output type=local,dest=/tmp/slc .   # per side, from its checkout
export SLC_TARBALL=/tmp/slc/lc-rs.tar.gz
cargo build --release -p bench-udfs                                    # target/release/libbench_udfs.so
cargo run --release -p udf-bench -- run [--profile quick|full] [--rows N] [--filter set_] [--keep] [--udf-debug 172.17.0.1:5055]
cargo run --release -p udf-bench -- compare --base a.json b.json --change c.json d.json
cargo run --release -p udf-bench -- show bench-results/<commit>-<timestamp>.json
```

Docker mode boots `exasol/docker-db` (`EXASOL_VERSION`, `EXA_DB_MEM_SIZE`, default 4 GiB).
External mode: `EXASOL_HOST`, `EXASOL_PORT`, `BUCKETFS_PORT`, `BUCKETFS_PASSWORD`.
Results land in the gitignored `bench-results/` as `<commit>[-dirty]-<timestamp>.json`.
`--udf-debug host:port` adds `%udf_debug_level debug` to every script and redirects the
runtime log to a TCP listener on the host (`python3 -c` or `nc -l`); count `send mt=9`
lines per cell to calibrate Tier 1's `BENCH_ROWS_PER_CYCLE` (frames per `MT_RUN` cycle × rows).

| `--profile` | table rows (`strblock` table / `wide` rows) | warm-up | reps | 44 cells on a running DB (same machine, Docker, 4 GiB) | band |
|---|---|---|---|---|---|
| `quick` (default) | 250,000 (2,500 / 62,500) | 1 | 3 | under 2 min (86 s including the Docker start) | ±15 % |
| `full` | 1,000,000 (10,000 / 250,000) | 1 | 5 | about 4.5 min (264 s including the Docker start) | ±8 % |

The seven cells that feed `strblock` table rows into a UDF (`control_strblock`,
`scalar_returns_strblock`, `set_returns_strblock_*`, `set_emits_strblock_*`) run at 3,000 to
7,000 rows/s on docker-db 2026.1.1 (measured: builtin Python3 over the same table is equally
slow, and the runtime debug log shows the client idle waiting for `MT_NEXT`). At the full row
count they took 25 of a 26-minute `quick` run while measuring the engine, so their source table
is `n / 100` rows (`STRBLOCK_INPUT_DIVISOR` in `cells.rs`, floor 1,000 so the 1,000-group
cells keep every group). Their `rows_per_s` stays comparable across runs; do not read their
`x_ctrl` ratio against the native cells. The generated `*_strblock_*` cells (`scalar_emits_gen`,
`set_gen`) do not read the table and keep the full `n`.

The `wide` generator cells emit `n / 4` rows (`WIDE_GEN_DIVISOR` in `cells.rs`): at 472 bytes
a row they still move about twice the bytes of every other cell put together, and the full
`n` would cost the run another two minutes. Generator cells with a measured `bytes/row`
report `MB_per_s` (wire megabytes per second) next to `rows_per_s`; the per-class constant
`WIRE_BYTES_PER_ROW` in `cells.rs` comes from Tier 1's counter table and must be updated when
a generator or the encoder changes.

Cells: `control_<class>`, `scalar_returns_<class>`, `scalar_emits_gen_<class>_<mode>[_noemit]`
(`wide` modes: `row`, `batch8k`, `batch64k`; no `batch64k_noemit`), `scalar_emits_pt`,
`set_returns_<class>_g<G>`, `set_emits_<class>_<mode>_g<G>`, `set_gen_<class>_<mode>`.
Every query returns at most one row; table-driven cells also report `ratio_to_control`.
`scalar_emits_pt` reports `incorrect` instead of a time while emitted rows do not land
beside their input rows.

A/B protocol against one external DB:

1. Build tarball and `.so` from the base checkout, `run` (file A).
2. Same from the change checkout, `run` (file C).
3. Repeat both once more (files B, D) so `compare` pools ten samples a side.
4. `compare --base A B --change C D`; mixed profiles are refused.

```
cell                     base_ms  base_min  chg_ms  chg_min  delta [95% CI]           verdict   flags
scalar_returns_native      132.8     131.9   118.0    117.1  -11.2% [-13.0%, -9.3%]   improved
```

| interval vs 0 | median delta vs band | verdict |
|---|---|---|
| excludes | outside | `improved` / `regressed` |
| excludes | inside | `small` |
| crosses | any | `no change` |
| fewer than 8 pooled samples on a side | | verdict carries `low power` |

Tukey outliers are flagged, never removed.

## Decisions

| | |
|---|---|
| One bench UDF cdylib | `bench-udfs` serves both tiers; optional dependency of the runtime behind the `bench` feature (it needs `emit-arrow`; a plain dev-dependency would unify that feature into every test build). Not in the CI artifact allowlist. |
| Criterion is the Tier 1 A/B | `--save-baseline` / `--baseline`; the `.so` is rebuilt from the same tree so the ABI fingerprint always matches. |
| Tier 2 compares runs, not SLCs | each side builds its own tarball and `.so`; drift is absorbed by control ratios and alternating runs. |
| No BIGINT | BIGINT travels as a decimal string; keys are `DECIMAL(18,0)` so `native` is native on the wire. |
| `wide` is synthetic and emit-only | no Parquet reader in the bench UDF (that would measure the reader and BucketFS, not the SLC); batches are built in memory from cheap deterministic generators, and the schema lives once in `bench-schema` so the UDF, the mock engine and the driver cannot drift. |
| CI runs Tier 1 smoke only | no performance gate. |
| Quick is the loop, full is the evidence | a performance PR quotes both tiers in the full profile. |
