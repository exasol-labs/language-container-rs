#!/usr/bin/env bash
#
# about_toml_test.sh — config-assertion tests for the repo-root about.toml
# (the cargo-about config driving dist/generate-licenses.sh's Rust-crate
# license bundle). Plain grep-based assertions: about.toml is static config,
# not a Cargo crate, so there is no `cargo test` target for it.
#
# Run: bash dist/tests/about_toml_test.sh
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
ABOUT_TOML="$ROOT/about.toml"

failures=0

fail() {
    echo "FAIL: $1"
    failures=$((failures + 1))
}

pass() {
    echo "PASS: $1"
}

# The shipped exaudfclient binary and the UDF cdylibs it loads are both glibc,
# and nothing musl is distributed, so the license scan unions only the two glibc
# (gnu) arches. Every shipped arch must be listed to avoid silently dropping a
# dependency's notice; musl must NOT be listed (it would over-attribute a
# platform we do not ship).
REQUIRED_TRIPLES=(
    "x86_64-unknown-linux-gnu"
    "aarch64-unknown-linux-gnu"
)
FORBIDDEN_TRIPLES=(
    "x86_64-unknown-linux-musl"
    "aarch64-unknown-linux-musl"
)

about_toml_lists_gnu_triples() {
    local targets_block
    targets_block="$(awk '/^targets = \[/{flag=1} flag{print} flag && /\]/{flag=0}' "$ABOUT_TOML")"
    if [[ -z "$targets_block" ]]; then
        fail "about_toml_lists_gnu_triples: no 'targets = [...]' array found in $ABOUT_TOML"
        return
    fi
    for triple in "${REQUIRED_TRIPLES[@]}"; do
        if [[ "$targets_block" != *"$triple"* ]]; then
            fail "about_toml_lists_gnu_triples: targets array missing '$triple'"
            return
        fi
    done
    for triple in "${FORBIDDEN_TRIPLES[@]}"; do
        if [[ "$targets_block" == *"$triple"* ]]; then
            fail "about_toml_lists_gnu_triples: targets array must not list musl triple '$triple' (nothing musl is shipped)"
            return
        fi
    done
    pass "about_toml_lists_gnu_triples"
}

about_toml_comments_glibc_rationale() {
    if ! grep -q "glibc" "$ABOUT_TOML"; then
        fail "about_toml_comments_glibc_rationale: no comment mentioning 'glibc' in $ABOUT_TOML"
        return
    fi
    if ! grep -qi "union" "$ABOUT_TOML"; then
        fail "about_toml_comments_glibc_rationale: no comment mentioning cargo-about's union-across-targets behavior in $ABOUT_TOML"
        return
    fi
    pass "about_toml_comments_glibc_rationale"
}

about_toml_lists_gnu_triples
about_toml_comments_glibc_rationale

if [[ "$failures" -gt 0 ]]; then
    echo "$failures test(s) failed"
    exit 1
fi
echo "All tests passed"
