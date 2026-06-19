#!/usr/bin/env bash
#
# generate-os-licenses.sh — produce dist/THIRD-PARTY-OS-LICENSES.md.
#
# Renders the OS/runtime attribution bundle for the distributed SLC tarball:
#   1. `cargo about` renders the preamble (dist/os-licenses.hbs) plus the
#      canonical text of every license named in dist/os-attribution/Cargo.toml,
#      pulled from cargo-about's embedded SPDX store (no committed license text).
#   2. The GCC Runtime Library Exception text is appended — cargo-about renders
#      base licenses but not `WITH <exception>` clauses — fetched from the SPDX
#      license-list-data so it, too, is not committed to the repo.
#
# The output is git-ignored (see dist/.gitignore); Dockerfile.alpine COPYs it
# into /exaudf so it ships inside lc-rs.tar.gz. Run locally to verify; CI runs it
# before `docker build`.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUT="$HERE/THIRD-PARTY-OS-LICENSES.md"
GCC_EXCEPTION_URL="https://raw.githubusercontent.com/spdx/license-list-data/main/text/GCC-exception-3.1.txt"

command -v cargo-about >/dev/null 2>&1 || {
    echo "error: cargo-about not found. Install: cargo install --locked cargo-about@0.9.0" >&2
    exit 1
}

cargo about generate \
    --manifest-path "$HERE/os-attribution/Cargo.toml" \
    -c "$HERE/about-os.toml" \
    "$HERE/os-licenses.hbs" \
    -o "$OUT"

# Append the GCC Runtime Library Exception (not emitted by cargo-about).
{
    printf '## GCC Runtime Library Exception 3.1 (GCC-exception-3.1)\n\n'
    printf 'Applies to libstdc++.so.6 and libgcc_s.so.1, in addition to GPL-3.0.\n\n'
    printf '```\n'
    curl -sSfL "$GCC_EXCEPTION_URL"
    printf '```\n'
} >> "$OUT"

echo "Generated $OUT"
