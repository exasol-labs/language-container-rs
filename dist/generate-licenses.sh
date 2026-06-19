#!/usr/bin/env bash
#
# generate-licenses.sh — produce the third-party license bundles shipped in
# lc-rs.tar.gz. Both outputs embed full license texts and are git-ignored
# (see dist/.gitignore); Dockerfile.alpine COPYs them into /exaudf. Run locally
# to verify; CI runs it before `docker build`.
#
#   dist/THIRD-PARTY-LICENSES.md     — Rust crate graph statically linked into
#                                      exaudfclient (cargo about + about.hbs).
#   dist/THIRD-PARTY-OS-LICENSES.md  — Alpine apk packages + copied glibc/GCC
#                                      runtime libs (cargo about over a synthetic
#                                      crate, + the appended GCC Runtime Library
#                                      Exception, which cargo about does not emit
#                                      for a `WITH` clause).
#
# License texts come from cargo-about's embedded SPDX store (and, for the GCC
# exception, the SPDX license-list-data), so no verbatim license text is committed.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
RUST_OUT="$HERE/THIRD-PARTY-LICENSES.md"
OS_OUT="$HERE/THIRD-PARTY-OS-LICENSES.md"
GCC_EXCEPTION_URL="https://raw.githubusercontent.com/spdx/license-list-data/main/text/GCC-exception-3.1.txt"

command -v cargo-about >/dev/null 2>&1 || {
    echo "error: cargo-about not found. Install: cargo install --locked cargo-about@0.9.0" >&2
    exit 1
}

# 1. Rust crate notices (exaudfclient dependency graph).
cargo about generate \
    --manifest-path "$ROOT/crates/exaudfclient/Cargo.toml" \
    -c "$ROOT/about.toml" \
    "$ROOT/about.hbs" \
    -o "$RUST_OUT"

# 2. OS/runtime notices (synthetic crate drives cargo-about over the bundled set).
cargo about generate \
    --manifest-path "$HERE/os-attribution/Cargo.toml" \
    -c "$HERE/about-os.toml" \
    "$HERE/os-licenses.hbs" \
    -o "$OS_OUT"

# Append the GCC Runtime Library Exception (not emitted by cargo-about).
{
    printf '## GCC Runtime Library Exception 3.1 (GCC-exception-3.1)\n\n'
    printf 'Applies to libstdc++.so.6 and libgcc_s.so.1, in addition to GPL-3.0.\n\n'
    printf '```\n'
    curl -sSfL "$GCC_EXCEPTION_URL"
    printf '```\n'
} >> "$OS_OUT"

echo "Generated $RUST_OUT and $OS_OUT"
