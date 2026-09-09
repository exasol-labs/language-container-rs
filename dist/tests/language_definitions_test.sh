#!/usr/bin/env bash
#
# language_definitions_test.sh — contract assertions for a
# build_info/language_definitions.json document against the Exasol
# database's language_definitions v2 metadata schema (issue #90).
# Path-addressed jq assertions, not substring matching, so a renamed root
# key or a re-typed field is caught instead of silently passing. Same
# plain-assertion style as dist/tests/about_toml_test.sh.
#
# Sole owner of the DB SLC v2 contract: both the pre-build source check (run
# directly on build_info/language_definitions.json) and the post-build
# artifact check (dist/tests/slc_tarball_test.sh, on the extracted tarball
# copy) delegate to this script, so the source and the shipped copy can
# never state different schemas.
#
# Run: bash dist/tests/language_definitions_test.sh <document.json>
set -uo pipefail

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

[[ $# -eq 1 ]] || die "usage: $(basename "$0") <language_definitions.json path>"
DOC="$1"
[[ -f "$DOC" ]] || die "no such file: $DOC"
command -v jq >/dev/null 2>&1 || die "jq not found — install jq"

language_definitions_document_is_valid_json() {
    if ! jq empty "$DOC" >/dev/null 2>&1; then
        fail "language_definitions_document_is_valid_json: $DOC is not valid JSON"
        return
    fi
    pass "language_definitions_document_is_valid_json"
}

language_definitions_declares_schema_version_2() {
    if ! jq -e '.schema_version == 2' "$DOC" >/dev/null 2>&1; then
        fail "language_definitions_declares_schema_version_2: $DOC does not declare schema_version 2"
        return
    fi
    pass "language_definitions_declares_schema_version_2"
}

language_definitions_root_key_is_language_definitions() {
    if jq -e 'has("languages")' "$DOC" >/dev/null 2>&1; then
        fail "language_definitions_root_key_is_language_definitions: $DOC still declares the old root key 'languages'"
        return
    fi
    if ! jq -e '.language_definitions | type == "array"' "$DOC" >/dev/null 2>&1; then
        fail "language_definitions_root_key_is_language_definitions: $DOC has no root 'language_definitions' array"
        return
    fi
    pass "language_definitions_root_key_is_language_definitions"
}

language_definitions_holds_one_conforming_definition() {
    if ! jq -e '.language_definitions | length == 1' "$DOC" >/dev/null 2>&1; then
        fail "language_definitions_holds_one_conforming_definition: .language_definitions does not hold exactly one definition"
        return
    fi
    if ! jq -e '.language_definitions[0].aliases == ["RUST"]' "$DOC" >/dev/null 2>&1; then
        fail "language_definitions_holds_one_conforming_definition: .language_definitions[0].aliases is not [\"RUST\"]"
        return
    fi
    if ! jq -e '.language_definitions[0].protocol == "localzmq+protobuf"' "$DOC" >/dev/null 2>&1; then
        fail "language_definitions_holds_one_conforming_definition: .language_definitions[0].protocol is not \"localzmq+protobuf\""
        return
    fi
    if ! jq -e '.language_definitions[0].parameters == [{"key": "lang", "value": "rust"}]' "$DOC" >/dev/null 2>&1; then
        fail "language_definitions_holds_one_conforming_definition: .language_definitions[0].parameters is not [{\"key\": \"lang\", \"value\": \"rust\"}]"
        return
    fi
    if ! jq -e '.language_definitions[0].udf_client_path.executable == "/exaudf/exaudfclient"' "$DOC" >/dev/null 2>&1; then
        fail "language_definitions_holds_one_conforming_definition: .language_definitions[0].udf_client_path.executable is not \"/exaudf/exaudfclient\""
        return
    fi
    # jq renders an absent key and a null value identically, so presence and
    # value must be asserted as two separate checks — has("deprecation")
    # alone would still pass a document that never mentions the key.
    if ! jq -e '.language_definitions[0] | has("deprecation")' "$DOC" >/dev/null 2>&1; then
        fail "language_definitions_holds_one_conforming_definition: .language_definitions[0] has no 'deprecation' key"
        return
    fi
    if ! jq -e '.language_definitions[0].deprecation == null' "$DOC" >/dev/null 2>&1; then
        fail "language_definitions_holds_one_conforming_definition: .language_definitions[0].deprecation is not null"
        return
    fi
    pass "language_definitions_holds_one_conforming_definition"
}

language_definitions_definition_has_no_arguments_key() {
    if ! jq -e '.language_definitions[0] != null' "$DOC" >/dev/null 2>&1; then
        fail "language_definitions_definition_has_no_arguments_key: .language_definitions[0] is missing"
        return
    fi
    if jq -e '.language_definitions[0] | has("arguments")' "$DOC" >/dev/null 2>&1; then
        fail "language_definitions_definition_has_no_arguments_key: .language_definitions[0] still declares the old 'arguments' key"
        return
    fi
    pass "language_definitions_definition_has_no_arguments_key"
}

# Every assertion below addresses the document by jq key path, and jq reports
# an unparseable document the same way it reports an absent key. Running them
# on a document that does not parse would turn one syntax error into four
# messages naming fields the document may well declare correctly, so they run
# only once the document is known to be valid JSON.
language_definitions_document_is_valid_json
if [[ "$failures" -eq 0 ]]; then
    language_definitions_declares_schema_version_2
    language_definitions_root_key_is_language_definitions
    language_definitions_holds_one_conforming_definition
    language_definitions_definition_has_no_arguments_key
fi

if [[ "$failures" -gt 0 ]]; then
    echo "$failures test(s) failed"
    exit 1
fi
echo "All tests passed"
