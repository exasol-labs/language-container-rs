#!/usr/bin/env bash
#
# slc_tarball_test.sh — contract assertions over the shipped SLC artifact
# tarball produced by `docker build --target artifact`. The extracted tree is
# the UDF's entire root filesystem, so every assertion here states something a
# UDF may rely on inside the sandbox. Same plain-assertion style as
# dist/tests/about_toml_test.sh and dist/tests/os_licenses_test.sh.
#
# Architecture-dependent facts (multiarch triplet, loader path, ELF machine)
# are derived from the runner and from the tree itself, never hardcoded, so the
# same script asserts the correct contract on x86_64 and aarch64.
#
# Run: bash dist/tests/slc_tarball_test.sh <tarball>
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
GLIBC_FLOOR_FILE="$ROOT/crates/cargo-exasol-udf/slc-glibc-floor.txt"

# The library surface a UDF may link dynamically instead of vendoring, by the
# name each library is documented under. The same committed file drives the
# container's staging loop and `cargo exasol-udf validate`, so this test cannot
# bless a surface either of them disagrees with. A name that differs from the
# library's own soname (libbz2.so.1's soname is libbz2.so.1.0) is covered from
# the other side by slc_tarball_dt_needed_closure_is_complete, which derives
# every soname from the staged files themselves.
LIBRARY_SURFACE_FILE="$ROOT/crates/cargo-exasol-udf/slc-library-surface.txt"

CLIENT_REL="exaudf/exaudfclient"

# exaudfclient's own dynamic dependencies, excluding the loader (which is
# derived from its PT_INTERP). zmq is statically linked via zeromq-src, and
# bzip2 is dropped entirely by dead-code elimination — exarrow-rs only reaches
# for it on the CSV IMPORT/EXPORT path this project never calls.
CLIENT_EXPECTED_DT_NEEDED=(
    libc.so.6
    libgcc_s.so.1
    libm.so.6
    libstdc++.so.6
)
CLIENT_FORBIDDEN_DT_NEEDED_PATTERNS=(
    'libbz2*'
    'libzmq*'
)

# Measured 16.1 MB on x86_64 (2026-08). The ceiling exists to catch a regression
# back to staging the donor's full library surface (~29 MB) or flattening its
# rootfs, so it must stay well below those; the measured value is printed on
# every run so ordinary drift stays visible.
STAGED_SURFACE_CEILING_BYTES=24000000

MAX_SYMLINK_HOPS=16

FORBIDDEN_PAYLOAD_PATHS=(
    bin/sh
    usr/bin/apt
    usr/bin/dpkg
)

NOTICE_FILES=(
    exaudf/LICENSE
    exaudf/THIRD-PARTY-LICENSES.md
    exaudf/THIRD-PARTY-OS-LICENSES.md
)

OS_NOTICE_FORBIDDEN_TERMS=(apk alpine aports musl)

failures=0

fail() {
    echo "FAIL: $1"
    failures=$((failures + 1))
}

pass() {
    echo "PASS: $1"
}

die() {
    echo "ERROR: $1" >&2
    exit 2
}

# --- tree helpers ------------------------------------------------------------

declare -A TREE_PATH_BY_BASENAME=()
declare -A SONAME_OF_FILE=()
declare -A NEEDED_OF_FILE=()
declare -a STAGED_ELF_FILES=()

# Follow a symlink chain the way the loader would inside the extracted tree: an
# absolute target names a path in the tree, not on the host running the test.
resolve_in_tree() {
    local path="$1" target hop
    for ((hop = 0; hop < MAX_SYMLINK_HOPS; hop++)); do
        if [[ ! -L "$path" ]]; then
            printf '%s\n' "$path"
            return 0
        fi
        target="$(readlink "$path")"
        if [[ "$target" == /* ]]; then
            path="$TREE$target"
        else
            path="$(dirname "$path")/$target"
        fi
    done
    return 1
}

# The loader finds a soname by looking the name itself up in the library
# directories, so a soname resolves only through a tree entry of that name —
# either the file or the link ldconfig wrote for it (libbz2.so.1's soname is
# libbz2.so.1.0). Resolving it back to the file that declares it would make
# every such check vacuously true.
resolve_soname() {
    local name="$1" candidate resolved
    candidate="${TREE_PATH_BY_BASENAME[$name]:-}"
    [[ -n "$candidate" ]] || return 1
    resolved="$(resolve_in_tree "$candidate")" || return 1
    [[ -f "$resolved" ]] || return 1
    printf '%s\n' "$resolved"
}

index_tree() {
    local path dynamic needed soname conf dir line existing real
    local -a lib_dirs=()
    local -A seen_dirs=()

    while IFS= read -r conf; do
        while IFS= read -r line; do
            dir="${line%%#*}"
            dir="${dir//[[:space:]]/}"
            [[ -n "$dir" ]] && lib_dirs+=("$TREE$dir")
        done <"$conf"
    done < <(find "$TREE/etc/ld.so.conf.d" -maxdepth 1 -name '*.conf' 2>/dev/null)
    lib_dirs+=("$(dirname "$TREE$LOADER_PATH")")

    for dir in "${lib_dirs[@]}"; do
        [[ -d "$dir" ]] || continue
        # ld.so.conf.d commonly lists a directory under both its usr-merge
        # symlink and its real usr/ path (etc/ld.so.conf.d/x86_64-linux-gnu.conf
        # lists /lib/x86_64-linux-gnu and /usr/lib/x86_64-linux-gnu, which the
        # /lib -> usr/lib symlink makes the same physical directory), so entries
        # are deduplicated by resolved path before a basename can collide.
        real="$(cd "$dir" && pwd -P)"
        [[ -n "${seen_dirs[$real]:-}" ]] && continue
        seen_dirs["$real"]=1
        while IFS= read -r path; do
            existing="${TREE_PATH_BY_BASENAME["${path##*/}"]:-}"
            if [[ -n "$existing" && "$existing" != "$path" ]]; then
                fail "index_tree: basename '${path##*/}' is staged at both '${existing#"$TREE/"}' and '${path#"$TREE/"}'"
                return
            fi
            TREE_PATH_BY_BASENAME["${path##*/}"]="$path"
        done < <(find "$dir/" -mindepth 1 -maxdepth 1 \( -type f -o -type l \))
    done

    while IFS= read -r path; do
        dynamic="$(readelf -d "$path" 2>/dev/null)"
        [[ "$dynamic" == *"(NEEDED)"* || "$dynamic" == *"(SONAME)"* ]] || continue
        needed="$(printf '%s\n' "$dynamic" | sed -n 's/.*(NEEDED).*\[\(.*\)\]/\1/p' | tr '\n' ' ')"
        soname="$(printf '%s\n' "$dynamic" | sed -n 's/.*(SONAME).*\[\(.*\)\]/\1/p')"
        STAGED_ELF_FILES+=("$path")
        NEEDED_OF_FILE["$path"]="$needed"
        if [[ -n "$soname" ]]; then
            SONAME_OF_FILE["$path"]="$soname"
        fi
    done < <(find "$TREE" -type f)
}

# Highest GLIBC_x.y version the given ELF *defines* (.gnu.version_d). readelf
# also prints those names in the version-symbols and version-needs sections, so
# the definition section is isolated first.
max_glibc_defined() {
    readelf -V "$1" 2>/dev/null \
        | awk '/^Version definition section/ {inside = 1; next}
               /^Version needs section/ {inside = 0}
               /^Version symbols section/ {inside = 0}
               inside' \
        | sed -n 's/.*Name: GLIBC_\([0-9][0-9.]*\).*/\1/p' \
        | sort -V | tail -1
}

# Highest GLIBC_x.y version the given ELF *references* (.gnu.version_r).
max_glibc_referenced() {
    readelf -V "$1" 2>/dev/null \
        | awk '/^Version needs section/ {inside = 1; next} inside' \
        | sed -n 's/.*Name: GLIBC_\([0-9][0-9.]*\).*/\1/p' \
        | sort -V | tail -1
}

version_le() {
    [[ "$(printf '%s\n%s\n' "$1" "$2" | sort -V | tail -1)" == "$2" ]]
}

# --- assertions --------------------------------------------------------------

slc_tarball_contains_executable_client() {
    local client="$TREE/$CLIENT_REL"
    if [[ ! -f "$client" ]]; then
        fail "slc_tarball_contains_executable_client: $CLIENT_REL is not a regular file in the tarball"
        return
    fi
    if [[ ! -x "$client" ]]; then
        fail "slc_tarball_contains_executable_client: $CLIENT_REL is not executable"
        return
    fi
    if [[ ! -s "$client" ]]; then
        fail "slc_tarball_contains_executable_client: $CLIENT_REL is empty"
        return
    fi
    pass "slc_tarball_contains_executable_client"
}

slc_client_dt_needed_is_expected_set() {
    local entry actual expected pattern
    for entry in ${NEEDED_OF_FILE["$TREE/$CLIENT_REL"]:-}; do
        for pattern in "${CLIENT_FORBIDDEN_DT_NEEDED_PATTERNS[@]}"; do
            if [[ "$entry" == $pattern ]]; then
                fail "slc_client_dt_needed_is_expected_set: $CLIENT_REL must not link '$entry' (matches forbidden '$pattern')"
                return
            fi
        done
    done

    actual="$(printf '%s\n' ${NEEDED_OF_FILE["$TREE/$CLIENT_REL"]:-} \
        | grep -vFx "${LOADER_SONAME}" | sort | tr '\n' ' ')"
    expected="$(printf '%s\n' "${CLIENT_EXPECTED_DT_NEEDED[@]}" | sort | tr '\n' ' ')"
    if [[ "$actual" != "$expected" ]]; then
        fail "slc_client_dt_needed_is_expected_set: $CLIENT_REL DT_NEEDED set (loader '$LOADER_SONAME' excluded) is
  actual:   $actual
  expected: $expected"
        return
    fi
    pass "slc_client_dt_needed_is_expected_set"
}

slc_client_matches_host_arch_and_loader_resolves() {
    local host_machine client_machine loader
    host_machine="$(readelf -h /proc/self/exe 2>/dev/null | sed -n 's/^ *Machine: *//p')"
    if [[ -z "$host_machine" ]]; then
        fail "slc_client_matches_host_arch_and_loader_resolves: cannot read the host ELF machine from /proc/self/exe"
        return
    fi
    client_machine="$(readelf -h "$TREE/$CLIENT_REL" 2>/dev/null | sed -n 's/^ *Machine: *//p')"
    if [[ "$client_machine" != "$host_machine" ]]; then
        fail "slc_client_matches_host_arch_and_loader_resolves: $CLIENT_REL is built for '$client_machine', runner is '$host_machine'"
        return
    fi

    if [[ "$LIBDIR_REL" != "usr/lib/$(uname -m)-linux-gnu" ]]; then
        fail "slc_client_matches_host_arch_and_loader_resolves: staged multiarch libdir is '$LIBDIR_REL', expected 'usr/lib/$(uname -m)-linux-gnu'"
        return
    fi

    loader="$(resolve_in_tree "$TREE$LOADER_PATH")" || {
        fail "slc_client_matches_host_arch_and_loader_resolves: PT_INTERP '$LOADER_PATH' is a symlink loop inside the tree"
        return
    }
    if [[ ! -f "$loader" ]]; then
        fail "slc_client_matches_host_arch_and_loader_resolves: PT_INTERP '$LOADER_PATH' does not resolve to a file inside the tree"
        return
    fi
    if [[ ! -x "$loader" ]]; then
        fail "slc_client_matches_host_arch_and_loader_resolves: staged loader '$LOADER_PATH' is not executable"
        return
    fi
    pass "slc_client_matches_host_arch_and_loader_resolves"
}

slc_tarball_usr_merge_symlinks_match_arch() {
    local name target required=(lib bin sbin)

    # /lib64 exists as a usr-merge symlink only where the loader lives there:
    # x86_64's PT_INTERP is /lib64/ld-linux-x86-64.so.2, aarch64 has no /lib64.
    case "$LOADER_PATH" in
    /lib64/*) required+=(lib64) ;;
    esac

    for name in lib lib64 bin sbin; do
        [[ -L "$TREE/$name" || -e "$TREE/$name" ]] || continue
        if [[ ! -L "$TREE/$name" ]]; then
            fail "slc_tarball_usr_merge_symlinks_match_arch: /$name is not a symlink — real files staged there would sit outside /usr"
            return
        fi
        target="$(readlink "$TREE/$name")"
        if [[ "$target" != "usr/$name" ]]; then
            fail "slc_tarball_usr_merge_symlinks_match_arch: /$name points at '$target', expected 'usr/$name'"
            return
        fi
        if [[ ! -d "$TREE/usr/$name" ]]; then
            fail "slc_tarball_usr_merge_symlinks_match_arch: /usr/$name is not a real directory behind the /$name link"
            return
        fi
    done

    for name in "${required[@]}"; do
        if [[ ! -L "$TREE/$name" ]]; then
            fail "slc_tarball_usr_merge_symlinks_match_arch: /$name is missing (expected a usr-merge symlink on this architecture)"
            return
        fi
    done
    pass "slc_tarball_usr_merge_symlinks_match_arch"
}

slc_tarball_has_no_shell_or_package_manager() {
    local path dir entries
    for path in "${FORBIDDEN_PAYLOAD_PATHS[@]}"; do
        if [[ -e "$TREE/$path" || -L "$TREE/$path" ]]; then
            fail "slc_tarball_has_no_shell_or_package_manager: tarball ships '$path'"
            return
        fi
    done
    for dir in usr/bin usr/sbin; do
        if [[ ! -d "$TREE/$dir" ]]; then
            fail "slc_tarball_has_no_shell_or_package_manager: /$dir is missing (the usr-merge link would dangle)"
            return
        fi
        entries="$(find "$TREE/$dir" -mindepth 1)"
        if [[ -n "$entries" ]]; then
            fail "slc_tarball_has_no_shell_or_package_manager: /$dir is not empty:
$entries"
            return
        fi
    done
    pass "slc_tarball_has_no_shell_or_package_manager"
}

slc_tarball_has_c_utf8_locale() {
    local locale_dir="$TREE/usr/lib/locale/C.utf8"
    if [[ ! -d "$locale_dir" ]]; then
        fail "slc_tarball_has_c_utf8_locale: usr/lib/locale/C.utf8 is missing — LANG=C.UTF-8 would not resolve in the sandbox"
        return
    fi
    if [[ ! -s "$locale_dir/LC_CTYPE" ]]; then
        fail "slc_tarball_has_c_utf8_locale: usr/lib/locale/C.utf8/LC_CTYPE is missing or empty"
        return
    fi
    pass "slc_tarball_has_c_utf8_locale"
}

slc_tarball_library_surface_present() {
    local soname module modules module_path
    local -a surface=()
    if [[ ! -f "$LIBRARY_SURFACE_FILE" ]]; then
        fail "slc_tarball_library_surface_present: committed surface file $LIBRARY_SURFACE_FILE is missing"
        return
    fi
    while IFS= read -r soname; do
        soname="${soname//[[:space:]]/}"
        [[ -n "$soname" ]] && surface+=("$soname")
    done <"$LIBRARY_SURFACE_FILE"
    if [[ "${#surface[@]}" -eq 0 ]]; then
        fail "slc_tarball_library_surface_present: $LIBRARY_SURFACE_FILE names no library"
        return
    fi

    for soname in "${surface[@]}"; do
        if ! resolve_soname "$soname" >/dev/null; then
            fail "slc_tarball_library_surface_present: documented library '$soname' does not resolve to a file in the tree"
            return
        fi
    done

    if [[ ! -s "$(resolve_in_tree "$TREE/$LIBDIR_REL/ossl-modules/legacy.so")" ]]; then
        fail "slc_tarball_library_surface_present: OpenSSL legacy provider $LIBDIR_REL/ossl-modules/legacy.so is missing"
        return
    fi

    # The donor's engine set is architecture-dependent (padlock is x86-only in
    # some builds), so the directory is asserted non-empty and fully resolvable
    # rather than against a fixed file list.
    for module in ossl-modules engines-3; do
        modules="$(find "$TREE/$LIBDIR_REL/$module" -mindepth 1 -name '*.so' 2>/dev/null)"
        if [[ -z "$modules" ]]; then
            fail "slc_tarball_library_surface_present: $LIBDIR_REL/$module contains no .so module"
            return
        fi
        while IFS= read -r module_path; do
            if [[ ! -s "$(resolve_in_tree "$module_path")" ]]; then
                fail "slc_tarball_library_surface_present: $module entry '${module_path#"$TREE/"}' does not resolve to a non-empty file"
                return
            fi
        done <<<"$modules"
    done
    pass "slc_tarball_library_surface_present"
}

slc_tarball_dt_needed_closure_is_complete() {
    local path entry soname
    if [[ "${#STAGED_ELF_FILES[@]}" -eq 0 ]]; then
        fail "slc_tarball_dt_needed_closure_is_complete: the tarball holds no dynamic ELF at all"
        return
    fi
    for path in "${STAGED_ELF_FILES[@]}"; do
        for entry in ${NEEDED_OF_FILE["$path"]}; do
            if ! resolve_soname "$entry" >/dev/null; then
                fail "slc_tarball_dt_needed_closure_is_complete: '${path#"$TREE/"}' needs '$entry', which does not resolve inside the tree"
                return
            fi
        done
        # A staged file whose own soname differs from its file name is only
        # loadable through the link ldconfig writes (libbz2.so.1's soname is
        # libbz2.so.1.0), so the soname must resolve too.
        soname="${SONAME_OF_FILE["$path"]:-}"
        if [[ -n "$soname" ]] && ! resolve_soname "$soname" >/dev/null; then
            fail "slc_tarball_dt_needed_closure_is_complete: '${path#"$TREE/"}' declares soname '$soname', which does not resolve inside the tree — did 'ldconfig -r' run?"
            return
        fi
    done
    pass "slc_tarball_dt_needed_closure_is_complete"
}

slc_tarball_nsswitch_modules_are_staged() {
    local conf="$TREE/etc/nsswitch.conf" modules module
    if [[ ! -s "$conf" ]]; then
        fail "slc_tarball_nsswitch_modules_are_staged: etc/nsswitch.conf is missing or empty"
        return
    fi
    modules="$(sed -E 's/#.*//; s/\[[^]]*\]//g' "$conf" \
        | awk -F: '/:/ {print $2}' | tr ' \t' '\n\n' | sort -u | sed '/^$/d')"
    if [[ -z "$modules" ]]; then
        fail "slc_tarball_nsswitch_modules_are_staged: etc/nsswitch.conf names no lookup module"
        return
    fi
    while IFS= read -r module; do
        if ! resolve_soname "libnss_$module.so.2" >/dev/null; then
            fail "slc_tarball_nsswitch_modules_are_staged: nsswitch.conf names '$module' but libnss_$module.so.2 is not staged"
            return
        fi
    done <<<"$modules"
    pass "slc_tarball_nsswitch_modules_are_staged"
}

slc_tarball_openssl_trust_path_resolves() {
    local trust_dir="$TREE/usr/lib/ssl" entry resolved bundle
    if [[ ! -d "$trust_dir" ]]; then
        fail "slc_tarball_openssl_trust_path_resolves: usr/lib/ssl is missing — OpenSSL's built-in default trust path"
        return
    fi
    if [[ ! -L "$trust_dir/cert.pem" ]]; then
        fail "slc_tarball_openssl_trust_path_resolves: usr/lib/ssl/cert.pem is not the donor's symlink into the certificate store"
        return
    fi
    while IFS= read -r entry; do
        resolved="$(resolve_in_tree "$entry")" || {
            fail "slc_tarball_openssl_trust_path_resolves: '${entry#"$TREE/"}' is a symlink loop"
            return
        }
        if [[ ! -e "$resolved" ]]; then
            fail "slc_tarball_openssl_trust_path_resolves: '${entry#"$TREE/"}' points outside the tree at '$(readlink "$entry")'"
            return
        fi
    done < <(find "$trust_dir" -mindepth 1 -maxdepth 1)

    bundle="$(resolve_in_tree "$trust_dir/cert.pem")"
    if ! grep -q 'BEGIN CERTIFICATE' "$bundle"; then
        fail "slc_tarball_openssl_trust_path_resolves: the staged CA bundle '${bundle#"$TREE/"}' holds no PEM certificate"
        return
    fi
    pass "slc_tarball_openssl_trust_path_resolves"
}

slc_tarball_glibc_floor_matches_committed_value() {
    local committed staged_libc staged_floor client_max
    if [[ ! -f "$GLIBC_FLOOR_FILE" ]]; then
        fail "slc_tarball_glibc_floor_matches_committed_value: committed floor file $GLIBC_FLOOR_FILE is missing"
        return
    fi
    committed="$(tr -d '[:space:]' <"$GLIBC_FLOOR_FILE")"
    staged_libc="$(resolve_soname libc.so.6)" || {
        fail "slc_tarball_glibc_floor_matches_committed_value: libc.so.6 is not staged"
        return
    }
    staged_floor="$(max_glibc_defined "$staged_libc")"
    client_max="$(max_glibc_referenced "$TREE/$CLIENT_REL")"
    echo "INFO: staged libc.so.6 defines up to GLIBC_${staged_floor:-<none>}; committed floor $committed; $CLIENT_REL references up to GLIBC_${client_max:-<none>}"

    if [[ -z "$staged_floor" ]]; then
        fail "slc_tarball_glibc_floor_matches_committed_value: staged libc.so.6 defines no GLIBC_x.y version"
        return
    fi
    if [[ "$staged_floor" != "$committed" ]]; then
        fail "slc_tarball_glibc_floor_matches_committed_value: staged libc.so.6 defines up to GLIBC_$staged_floor but $GLIBC_FLOOR_FILE commits $committed"
        return
    fi
    if [[ -z "$client_max" ]]; then
        fail "slc_tarball_glibc_floor_matches_committed_value: $CLIENT_REL references no GLIBC_x.y version — it cannot be a glibc-dynamic binary"
        return
    fi
    if ! version_le "$client_max" "$committed"; then
        fail "slc_tarball_glibc_floor_matches_committed_value: $CLIENT_REL references GLIBC_$client_max, above the staged floor $committed"
        return
    fi
    pass "slc_tarball_glibc_floor_matches_committed_value"
}

slc_tarball_language_definitions_well_formed() {
    local defs="$TREE/build_info/language_definitions.json" compact declared declaration
    local -a expected
    if [[ ! -s "$defs" ]]; then
        fail "slc_tarball_language_definitions_well_formed: build_info/language_definitions.json is missing or empty"
        return
    fi
    compact="$(tr -d ' \t\n' <"$defs")"
    expected=(
        '"schema_version":2'
        '"aliases":["RUST"]'
        '"protocol":"localzmq+protobuf"'
        '"language_identifier":"rust"'
        '"arguments":["lang=rust"]'
        '"executable":"/exaudf/exaudfclient"'
    )
    for declaration in "${expected[@]}"; do
        if [[ "$compact" != *"$declaration"* ]]; then
            fail "slc_tarball_language_definitions_well_formed: build_info/language_definitions.json does not declare $declaration"
            return
        fi
    done

    declared="$(printf '%s' "$compact" | sed -n 's/.*"executable":"\([^"]*\)".*/\1/p')"
    if [[ ! -x "$TREE$declared" ]]; then
        fail "slc_tarball_language_definitions_well_formed: declared executable '$declared' is not an executable file in the tree"
        return
    fi
    pass "slc_tarball_language_definitions_well_formed"
}

slc_tarball_staged_surface_within_ceiling() {
    local total payload staged
    total="$(du -sb "$TREE" 2>/dev/null | cut -f1)"
    payload="$(du -sb "$TREE/exaudf" 2>/dev/null | cut -f1)"
    if [[ -z "$total" || -z "$payload" ]]; then
        fail "slc_tarball_staged_surface_within_ceiling: cannot measure the extracted tree with 'du -sb'"
        return
    fi
    staged=$((total - payload))
    echo "INFO: staged surface outside exaudf/ is $staged bytes (ceiling $STAGED_SURFACE_CEILING_BYTES); whole tree $total bytes"
    if [[ "$staged" -gt "$STAGED_SURFACE_CEILING_BYTES" ]]; then
        fail "slc_tarball_staged_surface_within_ceiling: staged surface $staged bytes exceeds the committed ceiling $STAGED_SURFACE_CEILING_BYTES"
        return
    fi
    pass "slc_tarball_staged_surface_within_ceiling"
}

slc_tarball_conf_resolver_symlinks() {
    local name target
    for name in hosts resolv.conf; do
        if [[ ! -L "$TREE/etc/$name" ]]; then
            fail "slc_tarball_conf_resolver_symlinks: etc/$name is not a symlink — the DB's /conf copy would not be picked up"
            return
        fi
        target="$(readlink "$TREE/etc/$name")"
        if [[ "$target" != "/conf/$name" ]]; then
            fail "slc_tarball_conf_resolver_symlinks: etc/$name points at '$target', expected '/conf/$name'"
            return
        fi
    done
    pass "slc_tarball_conf_resolver_symlinks"
}

slc_tarball_tmp_is_empty_and_world_writable() {
    local entries mode
    if [[ ! -d "$TREE/tmp" ]]; then
        fail "slc_tarball_tmp_is_empty_and_world_writable: tmp/ is missing — the client sets HOME=/tmp and writes its traces there"
        return
    fi
    entries="$(find "$TREE/tmp" -mindepth 1)"
    if [[ -n "$entries" ]]; then
        fail "slc_tarball_tmp_is_empty_and_world_writable: tmp/ ships build-time content:
$entries"
        return
    fi
    mode="$(stat -c '%a' "$TREE/tmp")"
    if [[ "$mode" != "1777" ]]; then
        fail "slc_tarball_tmp_is_empty_and_world_writable: tmp/ mode is '$mode', expected 1777 — the sandbox would silently lose its diagnostics"
        return
    fi
    pass "slc_tarball_tmp_is_empty_and_world_writable"
}

slc_tarball_zoneinfo_is_regular_file() {
    local zone="$TREE/usr/share/zoneinfo/Europe/Berlin" resolved
    if [[ ! -e "$zone" && ! -L "$zone" ]]; then
        fail "slc_tarball_zoneinfo_is_regular_file: usr/share/zoneinfo/Europe/Berlin is missing"
        return
    fi
    resolved="$(resolve_in_tree "$zone")" || {
        fail "slc_tarball_zoneinfo_is_regular_file: usr/share/zoneinfo/Europe/Berlin is a symlink loop"
        return
    }
    if [[ ! -f "$resolved" || ! -s "$resolved" ]]; then
        fail "slc_tarball_zoneinfo_is_regular_file: usr/share/zoneinfo/Europe/Berlin does not resolve to a non-empty regular file"
        return
    fi
    pass "slc_tarball_zoneinfo_is_regular_file"
}

slc_tarball_carries_notice_bundles() {
    local notice
    for notice in "${NOTICE_FILES[@]}"; do
        if [[ ! -f "$TREE/$notice" ]]; then
            fail "slc_tarball_carries_notice_bundles: $notice is not a regular file in the tarball"
            return
        fi
        if [[ ! -s "$TREE/$notice" ]]; then
            fail "slc_tarball_carries_notice_bundles: $notice is empty"
            return
        fi
    done
    pass "slc_tarball_carries_notice_bundles"
}

slc_tarball_os_notice_has_no_apk_references() {
    local notice="$TREE/exaudf/THIRD-PARTY-OS-LICENSES.md" term
    if [[ ! -s "$notice" ]]; then
        fail "slc_tarball_os_notice_has_no_apk_references: exaudf/THIRD-PARTY-OS-LICENSES.md is missing or empty"
        return
    fi
    for term in "${OS_NOTICE_FORBIDDEN_TERMS[@]}"; do
        if grep -qi "$term" "$notice"; then
            fail "slc_tarball_os_notice_has_no_apk_references: exaudf/THIRD-PARTY-OS-LICENSES.md still references '$term'"
            return
        fi
    done
    pass "slc_tarball_os_notice_has_no_apk_references"
}

# --- runner ------------------------------------------------------------------

if [[ $# -ne 1 ]]; then
    echo "usage: bash dist/tests/slc_tarball_test.sh <tarball>" >&2
    exit 2
fi
TARBALL="$1"
[[ -f "$TARBALL" ]] || die "no such tarball: $TARBALL"
command -v readelf >/dev/null 2>&1 || die "readelf not found — install binutils"

TREE="$(mktemp -d)"
trap 'rm -rf "$TREE"' EXIT
# -p keeps the archived modes verbatim; without it an ordinary user's umask
# rewrites them and every mode assertion here would test the runner's umask
# instead of the artifact BucketFS extracts as root.
tar -xzpf "$TARBALL" -C "$TREE" || die "cannot extract $TARBALL"

slc_tarball_contains_executable_client

# Every remaining assertion reads the client or the index built from the tree,
# so a client that cannot be read at all stops the run instead of producing
# sixteen derived failures.
LOADER_PATH="$(readelf -l "$TREE/$CLIENT_REL" 2>/dev/null \
    | sed -n 's/.*interpreter: \(.*\)]/\1/p' | tr -d ' ')"
[[ -n "$LOADER_PATH" ]] || die "no PT_INTERP in $CLIENT_REL — cannot derive the loader path"
LOADER_SONAME="${LOADER_PATH##*/}"
LIBDIR_REL="$(cd "$TREE" && find usr/lib -maxdepth 1 -name '*-linux-gnu' -type d 2>/dev/null)"
[[ -n "$LIBDIR_REL" ]] || die "no usr/lib/<triplet> multiarch directory in the tarball"

index_tree

slc_client_dt_needed_is_expected_set
slc_client_matches_host_arch_and_loader_resolves
slc_tarball_usr_merge_symlinks_match_arch
slc_tarball_has_no_shell_or_package_manager
slc_tarball_has_c_utf8_locale
slc_tarball_library_surface_present
slc_tarball_dt_needed_closure_is_complete
slc_tarball_nsswitch_modules_are_staged
slc_tarball_openssl_trust_path_resolves
slc_tarball_glibc_floor_matches_committed_value
slc_tarball_language_definitions_well_formed
slc_tarball_staged_surface_within_ceiling
slc_tarball_conf_resolver_symlinks
slc_tarball_tmp_is_empty_and_world_writable
slc_tarball_zoneinfo_is_regular_file
slc_tarball_carries_notice_bundles
slc_tarball_os_notice_has_no_apk_references

if [[ "$failures" -gt 0 ]]; then
    echo "$failures test(s) failed"
    exit 1
fi
echo "All tests passed"
