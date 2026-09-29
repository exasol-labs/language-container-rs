#!/usr/bin/env bash
#
# Build every test-udfs/* fixture in release. The integration scenarios dlopen
# the resulting target/release/lib*.so; CI and scripts/ci-it-local.sh both call
# this so the fixture list cannot drift between them. A fixture's crate name
# must equal its directory name.
set -euo pipefail
cd "$(dirname "$0")/.."

pkgs=()
for dir in test-udfs/*/; do
  pkgs+=(-p "$(basename "$dir")")
done
cargo build --release "${pkgs[@]}"
