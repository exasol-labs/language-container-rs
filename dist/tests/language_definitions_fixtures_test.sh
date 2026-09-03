#!/usr/bin/env bash
#
# language_definitions_fixtures_test.sh — drives
# dist/tests/language_definitions_test.sh against one fixture per defect
# class, under dist/tests/fixtures/language_definitions/. Each negative
# fixture is an otherwise-conforming document varying exactly one fact, so a
# non-zero exit on it is evidence that the corresponding assertion in
# language_definitions_test.sh actually discriminates, not just that it
# exists. conforming.json proves the same script accepts a fully correct
# document. Same plain-assertion style as dist/tests/about_toml_test.sh.
#
# The driver owns these pass/fail assertions; language_definitions_test.sh
# keeps exactly one subject (the document passed as its argument) and is
# never edited to know about fixtures.
#
# Run: bash dist/tests/language_definitions_fixtures_test.sh
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CONTRACT_SCRIPT="$HERE/language_definitions_test.sh"
FIXTURES_DIR="$HERE/fixtures/language_definitions"

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

[[ -f "$CONTRACT_SCRIPT" ]] || die "missing contract script: $CONTRACT_SCRIPT"
[[ -d "$FIXTURES_DIR" ]] || die "missing fixtures dir: $FIXTURES_DIR"

# Exit 2 from the contract script means it could not run at all (usage error,
# missing file, missing jq) and is evidence of nothing, so it stops the driver
# instead of counting as a rejection. Exit 1 is a contract violation, but only
# the FAIL line naming the fact this fixture varies proves the intended
# assertion discriminates — a collateral failure from another assertion does
# not, so the expected message is matched against the FAIL lines themselves.
assert_rejected() {
    local name="$1" fixture="$2" expected_message="$3"
    local path="$FIXTURES_DIR/$fixture" output status line
    [[ -f "$path" ]] || die "missing fixture: $path"
    output="$(bash "$CONTRACT_SCRIPT" "$path" 2>&1)"
    status=$?
    if [[ "$status" -eq 0 ]]; then
        fail "$name: $fixture unexpectedly passed the contract script"
        return
    fi
    if [[ "$status" -ne 1 ]]; then
        die "$name: contract script could not run on $fixture (exit $status): $output"
    fi
    while IFS= read -r line; do
        if [[ "$line" == "FAIL: "*"$expected_message"* ]]; then
            pass "$name"
            return
        fi
    done <<<"$output"
    fail "$name: $fixture was rejected, but no FAIL line reported \"$expected_message\"; output was: $output"
}

assert_accepted() {
    local name="$1" fixture="$2"
    local path="$FIXTURES_DIR/$fixture" output status
    [[ -f "$path" ]] || die "missing fixture: $path"
    output="$(bash "$CONTRACT_SCRIPT" "$path" 2>&1)"
    status=$?
    if [[ "$status" -eq 2 ]]; then
        die "$name: contract script could not run on $fixture: $output"
    fi
    if [[ "$status" -ne 0 ]]; then
        fail "$name: $fixture unexpectedly failed the contract script: $output"
        return
    fi
    pass "$name"
}

# One case per line, in the order the contract script asserts them, so a
# missing branch is visible as a gap in the sequence.
assert_rejected "language_definitions_rejects_malformed" "malformed.json" "is not valid JSON"
assert_rejected "language_definitions_rejects_schema_version_wrong" "schema_version_wrong.json" "does not declare schema_version 2"
assert_rejected "language_definitions_rejects_root_key_renamed" "root_key_renamed.json" "still declares the old root key 'languages'"
assert_rejected "language_definitions_rejects_definitions_not_array" "definitions_not_array.json" "has no root 'language_definitions' array"
assert_rejected "language_definitions_rejects_two_definitions" "two_definitions.json" "does not hold exactly one definition"
assert_rejected "language_definitions_rejects_aliases_changed" "aliases_changed.json" ".language_definitions[0].aliases is not"
assert_rejected "language_definitions_rejects_protocol_changed" "protocol_changed.json" ".language_definitions[0].protocol is not"
assert_rejected "language_definitions_rejects_parameters_retyped" "parameters_retyped.json" ".language_definitions[0].parameters is not"
assert_rejected "language_definitions_rejects_executable_changed" "executable_changed.json" ".language_definitions[0].udf_client_path.executable is not"
assert_rejected "language_definitions_rejects_deprecation_removed" "deprecation_removed.json" "has no 'deprecation' key"
assert_rejected "language_definitions_rejects_deprecation_non_null" "deprecation_non_null.json" ".language_definitions[0].deprecation is not null"
assert_rejected "language_definitions_rejects_arguments_readded" "arguments_readded.json" "still declares the old 'arguments' key"
assert_accepted "language_definitions_accepts_conforming" "conforming.json"

if [[ "$failures" -gt 0 ]]; then
    echo "$failures test(s) failed"
    exit 1
fi
echo "All tests passed"
