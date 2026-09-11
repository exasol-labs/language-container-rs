#!/usr/bin/env bash
# Build the SCALAR EMITS benchmark UDF .so against the v0.24.0 SDK from crates.io.
#
# The resulting .so has ABI version 7 (matching the v0.24.0 SLC runtime) and
# identical UDF logic to the current scalar-emit-bench-udf crate.
#
# Usage:
#   ./benches/scalar-emit-bench/build-old-udf.sh
#   export BENCH_UDF_SO_OLD=<path printed by the script>
set -euo pipefail

WORK_DIR="${TMPDIR:-/tmp}/scalar-emit-bench-old-udf"
rm -rf "$WORK_DIR"
mkdir -p "$WORK_DIR/src"

cat > "$WORK_DIR/Cargo.toml" <<'TOML'
[package]
name = "scalar-emit-bench-udf"
version = "0.1.0"
edition = "2021"

[lib]
name = "scalar_emit_bench_udf"
crate-type = ["cdylib"]

[dependencies]
exasol-udf-sdk = "=0.24.0"
exasol-udf-macros = "=0.24.0"
chrono = "0.4"
TOML

# Pin the same rustc that v0.24.0 used to build the SLC, so the SDK
# fingerprint (which includes the rustc version hash) matches at dlopen.
cat > "$WORK_DIR/rust-toolchain.toml" <<'TOML'
[toolchain]
channel = "1.94"
TOML

# Copy the current UDF source and patch the emit API:
# v0.25.0: ctx.emit(vec![...])   →  v0.24.0: ctx.emit(&[...])
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
SOURCE="$REPO_ROOT/benches/scalar-emit-bench-udf/src/lib.rs"

sed 's/ctx\.emit(vec!\[/ctx.emit(\&[/g' "$SOURCE" > "$WORK_DIR/src/lib.rs"

echo "[build-old-udf] Building v0.24.0-compatible UDF .so in $WORK_DIR ..."
cd "$WORK_DIR"
cargo build --release 2>&1

SO_PATH="$WORK_DIR/target/release/libscalar_emit_bench_udf.so"
if [ ! -f "$SO_PATH" ]; then
    echo "ERROR: expected .so not found at $SO_PATH" >&2
    exit 1
fi

echo ""
echo "=== v0.24.0-compatible UDF .so built ==="
echo "export BENCH_UDF_SO_OLD=$SO_PATH"
