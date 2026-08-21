#!/usr/bin/env bash
#
# os_licenses_test.sh — assertion tests for the OS-layer license notice
# bundle (dist/about-os.toml, dist/os-attribution/Cargo.toml,
# dist/os-licenses.hbs, dist/generate-licenses.sh, and the manifest they
# produce, dist/THIRD-PARTY-OS-LICENSES.md). Plain grep-based assertions,
# same style as dist/tests/about_toml_test.sh.
#
# Run: bash dist/tests/os_licenses_test.sh
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
ABOUT_OS_TOML="$ROOT/dist/about-os.toml"
CARGO_TOML="$ROOT/dist/os-attribution/Cargo.toml"
HBS="$ROOT/dist/os-licenses.hbs"
GENERATE_SCRIPT="$ROOT/dist/generate-licenses.sh"
OS_MANIFEST="$ROOT/dist/THIRD-PARTY-OS-LICENSES.md"

failures=0

fail() {
    echo "FAIL: $1"
    failures=$((failures + 1))
}

pass() {
    echo "PASS: $1"
}

os_boilerplate_committed_and_license_sets_agree() {
    local name
    for name in "$ABOUT_OS_TOML" "$CARGO_TOML" "$HBS" "$GENERATE_SCRIPT"; do
        if [[ ! -f "$name" ]]; then
            fail "os_boilerplate_committed_and_license_sets_agree: missing committed file $name"
            return
        fi
    done

    local license_line license_expr expr_atoms accepted_atoms
    license_line="$(grep '^license = ' "$CARGO_TOML")"
    if [[ -z "$license_line" ]]; then
        fail "os_boilerplate_committed_and_license_sets_agree: no 'license = \"...\"' line in $CARGO_TOML"
        return
    fi
    license_expr="$(echo "$license_line" | sed -E 's/^license = "(.*)"$/\1/')"
    expr_atoms="$(echo "$license_expr" | sed -E 's/ AND /\n/g' | sed -E 's/^\(//; s/\)$//' | sort)"

    accepted_atoms="$(awk '/^accepted = \[/{flag=1; next} /\]/{flag=0} flag{print}' "$ABOUT_OS_TOML" \
        | sed -E 's/^[[:space:]]*"(.*)",?[[:space:]]*$/\1/' | sort)"
    if [[ -z "$accepted_atoms" ]]; then
        fail "os_boilerplate_committed_and_license_sets_agree: no 'accepted = [...]' array found in $ABOUT_OS_TOML"
        return
    fi

    if [[ "$expr_atoms" != "$accepted_atoms" ]]; then
        fail "os_boilerplate_committed_and_license_sets_agree: license expression in $CARGO_TOML and accepted list in $ABOUT_OS_TOML name different sets:
--- Cargo.toml license expression ---
$expr_atoms
--- about-os.toml accepted list ---
$accepted_atoms"
        return
    fi

    for forbidden in "GPL-2.0-only" "BSD-2-Clause"; do
        if [[ "$expr_atoms" == *"$forbidden"* ]]; then
            fail "os_boilerplate_committed_and_license_sets_agree: $CARGO_TOML still names dropped license '$forbidden'"
            return
        fi
    done

    pass "os_boilerplate_committed_and_license_sets_agree"
}

os_manifest_covers_staged_library_set() {
    if [[ ! -f "$OS_MANIFEST" ]]; then
        fail "os_manifest_covers_staged_library_set: $OS_MANIFEST does not exist — run 'bash dist/generate-licenses.sh' first"
        return
    fi

    local staged_markers=(
        "libc.so.6"
        "libstdc++.so.6"
        "libssl.so.3"
        "libz.so.1"
        "libzstd.so.1"
        "libbz2.so.1"
        "ca-certificates"
        "tzdata"
    )
    local marker
    for marker in "${staged_markers[@]}"; do
        if ! grep -qF "$marker" "$OS_MANIFEST"; then
            fail "os_manifest_covers_staged_library_set: $OS_MANIFEST missing staged-library entry '$marker'"
            return
        fi
    done

    if ! grep -q "Debian 13" "$OS_MANIFEST"; then
        fail "os_manifest_covers_staged_library_set: $OS_MANIFEST does not name Debian 13"
        return
    fi
    if ! grep -q "snapshot.debian.org" "$OS_MANIFEST" || ! grep -q "sources.debian.org" "$OS_MANIFEST"; then
        fail "os_manifest_covers_staged_library_set: $OS_MANIFEST missing the Debian source-offer URLs"
        return
    fi

    if ! grep -qi "GCC Runtime Library Exception" "$OS_MANIFEST"; then
        fail "os_manifest_covers_staged_library_set: $OS_MANIFEST missing the GCC Runtime Library Exception text"
        return
    fi
    if ! grep -qF "bzip2 and libbzip2 License v1.0.6 (bzip2-1.0.6)" "$OS_MANIFEST"; then
        fail "os_manifest_covers_staged_library_set: $OS_MANIFEST missing bzip2 license text"
        return
    fi
    if ! grep -qF "Julian R Seward" "$OS_MANIFEST"; then
        fail "os_manifest_covers_staged_library_set: $OS_MANIFEST missing bzip2 license text"
        return
    fi

    local forbidden
    for forbidden in "apk" "alpine" "musl" "aports"; do
        if grep -qi "$forbidden" "$OS_MANIFEST"; then
            fail "os_manifest_covers_staged_library_set: $OS_MANIFEST still references '$forbidden'"
            return
        fi
    done

    pass "os_manifest_covers_staged_library_set"
}

os_boilerplate_committed_and_license_sets_agree
os_manifest_covers_staged_library_set

if [[ "$failures" -gt 0 ]]; then
    echo "$failures test(s) failed"
    exit 1
fi
echo "All tests passed"
