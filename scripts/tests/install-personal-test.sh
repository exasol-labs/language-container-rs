#!/usr/bin/env bash
# Sourced-function assertions for the Exasol Personal install path.
# Architecture-independent: registration-string assembly, entry preservation
# and deployment-descriptor parsing only — no VM, no database, no Docker.
set -uo pipefail

TESTS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCRIPTS_DIR="$(cd "$TESTS_DIR/.." && pwd)"

command -v jq >/dev/null 2>&1 || {
  echo "error: jq is required to run these tests" >&2
  exit 1
}

# shellcheck source=../lib/script_languages.sh
source "$SCRIPTS_DIR/lib/script_languages.sh"
# The Personal transport now lives in install.sh (behind --deployment); sourcing
# it defines the functions without running an install (guarded by BASH_SOURCE).
# shellcheck source=../install.sh
source "$SCRIPTS_DIR/install.sh"
set +e

INSTALL_SH_ENTRY='RUST=localzmq+protobuf:///bfsdefault/default/slc/rustslc?lang=rust#buckets/bfsdefault/default/slc/rustslc/exaudf/exaudfclient'
PERSONAL_ENTRY='RUST=localzmq+protobuf:///bfsdefault/default/rustslc?lang=rust#buckets/bfsdefault/default/rustslc/exaudf/exaudfclient'

FAILED=0

check() { # check <what> <expected> <actual>
  if [[ "$3" == "$2" ]]; then
    printf '  ok   %s\n' "$1"
  else
    printf '  FAIL %s\n         expected: [%s]\n         actual:   [%s]\n' "$1" "$2" "$3"
    FAILED=1
  fi
}

fragment_points_at_executable_no_leading_slash() {
  local entry fragment

  entry="$(script_languages_entry bfsdefault default slc/rustslc)"
  check "the install.sh layout assembles the historical registration string" \
    "$INSTALL_SH_ENTRY" "$entry"

  entry="$(script_languages_entry bfsdefault default rustslc)"
  check "the Personal layout assembles a bucket-root registration string" \
    "$PERSONAL_ENTRY" "$entry"

  fragment="${entry#*#}"
  check "the fragment names the exaudfclient executable, not its directory" \
    "buckets/bfsdefault/default/rustslc/exaudf/exaudfclient" "$fragment"
  check "the fragment has no leading slash" \
    "buckets" "${fragment%%/*}"

  script_languages_entry bfsdefault default "" >/dev/null 2>&1
  check "an empty path component is rejected instead of yielding a broken fragment" \
    "1" "$?"
}

preserves_existing_script_languages() {
  check "an empty parameter yields the RUST entry alone" \
    "$PERSONAL_ENTRY" \
    "$(script_languages_with_rust_entry "" "$PERSONAL_ENTRY")"

  check "pre-existing entries are kept and RUST is appended" \
    "PYTHON3=builtin_python3 JAVA=builtin_java $PERSONAL_ENTRY" \
    "$(script_languages_with_rust_entry "PYTHON3=builtin_python3 JAVA=builtin_java" "$PERSONAL_ENTRY")"

  check "a stale RUST entry is replaced rather than duplicated" \
    "PYTHON3=builtin_python3 JAVA=builtin_java $PERSONAL_ENTRY" \
    "$(script_languages_with_rust_entry \
      "PYTHON3=builtin_python3 RUST=localzmq+protobuf:///old/default/rustslc?lang=rust#buckets/old/default/rustslc/exaudf/exaudfclient JAVA=builtin_java" \
      "$PERSONAL_ENTRY")"

  check "surrounding and repeated whitespace is normalised" \
    "PYTHON3=builtin_python3 JAVA=builtin_java $PERSONAL_ENTRY" \
    "$(script_languages_with_rust_entry "  PYTHON3=builtin_python3   JAVA=builtin_java  " "$PERSONAL_ENTRY")"
}

reads_ssh_port_from_deployment_json() {
  local dir
  dir="$(mktemp -d)"

  printf '{"connection":{"host":"127.0.0.1","sshPort":52341,"port":8563}}\n' >"$dir/deployment.json"
  check "the SSH port comes from connection.sshPort" \
    "52341" "$(deployment_ssh_port "$dir")"

  printf '{"connection":{"host":"127.0.0.1","sshPort":52999,"port":8563}}\n' >"$dir/deployment.json"
  check "a reassigned SSH port is picked up on the next read, never cached" \
    "52999" "$(deployment_ssh_port "$dir")"

  check "the node key is located inside the deployment directory" \
    "$dir/local/node_access.pem" "$(deployment_key_path "$dir")"

  printf '{"connection":{"host":"127.0.0.1"}}\n' >"$dir/deployment.json"
  deployment_ssh_port "$dir" >/dev/null 2>&1
  check "a descriptor without connection.sshPort fails" "1" "$?"

  rm -f "$dir/deployment.json"
  deployment_ssh_port "$dir" >/dev/null 2>&1
  check "a missing descriptor fails" "1" "$?"

  rm -rf "$dir"
}

parses_current_script_languages_from_query_output() {
  check "a header-only result reads as an unset parameter" \
    "" "$(parse_script_languages "CURRENT_SCRIPT_LANGUAGES")"

  check "a plain result row is the current parameter value" \
    "PYTHON3=builtin_python3 JAVA=builtin_java" \
    "$(parse_script_languages "CURRENT_SCRIPT_LANGUAGES
PYTHON3=builtin_python3 JAVA=builtin_java")"

  check "a quoted result row is unquoted" \
    'PYTHON3=a,b JAVA=builtin_java' \
    "$(parse_script_languages 'CURRENT_SCRIPT_LANGUAGES
"PYTHON3=a,b JAVA=builtin_java"')"

  parse_script_languages "CURRENT_SCRIPT_LANGUAGES
some unexpected banner" >/dev/null 2>&1
  check "output that is not a list of ALIAS=… entries fails loudly" "1" "$?"
}

reset_connection_globals() {
  HOST=""
  PORT=8563
  USER=sys
  CLI_PORT=""
  CLI_USER=""
  PASSWORD=""
  BFS_PASSWORD=""
  SCOPE=SESSION
}

selects_transport_from_backend() {
  local dir backend

  dir="$(mktemp -d)"

  printf '{"backend":"local","connection":{"host":"127.0.0.1","dbPort":8563,"username":"sys"}}\n' \
    >"$dir/deployment.json"
  backend="$(deployment_backend "$dir")"
  check "a local descriptor reports the local backend" "local" "$backend"

  printf '{"backend":"aws","connection":{"host":"h.example","dbPort":8563,"username":"sys"}}\n' \
    >"$dir/deployment.json"
  backend="$(deployment_backend "$dir")"
  check "a cloud descriptor reports its cloud backend name" "aws" "$backend"

  printf '{"connection":{"host":"h.example"}}\n' >"$dir/deployment.json"
  deployment_backend "$dir" >/dev/null 2>&1
  check "a descriptor with no .backend field fails" "1" "$?"

  printf '{"backend":"","connection":{"host":"h.example"}}\n' >"$dir/deployment.json"
  deployment_backend "$dir" >/dev/null 2>&1
  check "a descriptor with an empty .backend field fails" "1" "$?"

  rm -rf "$dir"
}

resolves_cloud_connection_from_descriptor() {
  local dir rc

  dir="$(mktemp -d)"
  printf '{"backend":"aws","connection":{"host":"h.example","dbPort":8563,"username":"sys"}}\n' \
    >"$dir/deployment.json"
  printf '{"dbPassword":"secret"}\n' >"$dir/secrets.json"

  reset_connection_globals
  BFS_PASSWORD="bfspw"
  resolve_cloud_connection "$dir" >/dev/null 2>&1
  rc=$?

  check "resolve_cloud_connection succeeds given a complete cloud descriptor" "0" "$rc"
  check "HOST resolves from connection.host" "h.example" "$HOST"
  check "PORT resolves from connection.dbPort" "8563" "$PORT"
  check "USER resolves from connection.username" "sys" "$USER"
  check "PASSWORD resolves from secrets.json .dbPassword" "secret" "$PASSWORD"

  rm -rf "$dir"
}

cli_flags_override_cloud_descriptor() {
  local dir rc

  dir="$(mktemp -d)"
  printf '{"backend":"aws","connection":{"host":"h.example","dbPort":8563,"username":"sys"}}\n' \
    >"$dir/deployment.json"
  printf '{"dbPassword":"secret"}\n' >"$dir/secrets.json"

  # Simulates an operator who passed --host/--port/--user/--password: CLI_PORT
  # and CLI_USER are pre-set exactly as main's arg loop would set them. The
  # arg-loop capture itself (--port/--user -> CLI_PORT/CLI_USER) is exercised
  # only by the manual cloud run — the sourced harness never runs main's loop.
  reset_connection_globals
  HOST="override.example"
  CLI_PORT="1234"
  CLI_USER="admin"
  PASSWORD="overridepw"
  BFS_PASSWORD="bfspw"
  resolve_cloud_connection "$dir" >/dev/null 2>&1
  rc=$?

  check "resolve_cloud_connection succeeds with CLI overrides" "0" "$rc"
  check "an explicit --host wins over connection.host" "override.example" "$HOST"
  check "an explicit --port wins over connection.dbPort" "1234" "$PORT"
  check "an explicit --user wins over connection.username" "admin" "$USER"
  check "an explicit --password wins over secrets.json .dbPassword" "overridepw" "$PASSWORD"

  rm -rf "$dir"
}

cloud_requires_operator_bfs_password() {
  local dir err rc names_bfs_password

  dir="$(mktemp -d)"
  printf '{"backend":"aws","connection":{"host":"h.example","dbPort":8563,"username":"sys"}}\n' \
    >"$dir/deployment.json"
  printf '{"dbPassword":"secret"}\n' >"$dir/secrets.json"

  reset_connection_globals
  err="$(resolve_cloud_connection "$dir" 2>&1 >/dev/null)"
  rc=$?

  check "an empty --bfs-password fails cloud resolution" "1" "$rc"
  if [[ "$err" == *"--bfs-password"* ]]; then names_bfs_password=1; else names_bfs_password=0; fi
  check "the error names --bfs-password as the missing Personal requirement" "1" "$names_bfs_password"

  rm -rf "$dir"
}

cloud_requires_db_password() {
  local dir rc

  dir="$(mktemp -d)"
  printf '{"backend":"aws","connection":{"host":"h.example","dbPort":8563,"username":"sys"}}\n' \
    >"$dir/deployment.json"
  # No secrets.json: the deployment never provisioned a DB password, and no
  # --password override is supplied either.

  reset_connection_globals
  BFS_PASSWORD="bfspw"
  resolve_cloud_connection "$dir" >/dev/null 2>&1
  rc=$?

  check "an absent secrets.json with no --password override fails cloud resolution" "1" "$rc"

  rm -rf "$dir"
}

resolves_cloud_defaults_when_connection_fields_absent() {
  local dir rc

  dir="$(mktemp -d)"
  printf '{"backend":"aws","connection":{"host":"h.example"}}\n' \
    >"$dir/deployment.json"
  printf '{"dbPassword":"secret"}\n' >"$dir/secrets.json"

  reset_connection_globals
  BFS_PASSWORD="bfspw"
  resolve_cloud_connection "$dir" >/dev/null 2>&1
  rc=$?

  check "resolve_cloud_connection succeeds with dbPort/username absent" "0" "$rc"
  check "PORT falls back to 8563 when connection.dbPort is absent" "8563" "$PORT"
  check "USER falls back to sys when connection.username is absent" "sys" "$USER"

  rm -rf "$dir"
}

cloud_leaves_scope_untouched() {
  local dir

  dir="$(mktemp -d)"
  printf '{"backend":"aws","connection":{"host":"h.example","dbPort":8563,"username":"sys"}}\n' \
    >"$dir/deployment.json"
  printf '{"dbPassword":"secret"}\n' >"$dir/secrets.json"

  reset_connection_globals
  BFS_PASSWORD="bfspw"
  resolve_cloud_connection "$dir" >/dev/null 2>&1

  # Unlike the local transport, which forces SCOPE=SYSTEM, cloud resolution
  # MUST NOT touch SCOPE: cloud honors --scope, default SESSION.
  check "cloud resolution leaves SCOPE at its default (MUST NOT force SYSTEM)" \
    "SESSION" "$SCOPE"

  rm -rf "$dir"
}

run() {
  printf '%s\n' "$1"
  "$1"
}

run fragment_points_at_executable_no_leading_slash
run preserves_existing_script_languages
run reads_ssh_port_from_deployment_json
run parses_current_script_languages_from_query_output
run selects_transport_from_backend
run resolves_cloud_connection_from_descriptor
run cli_flags_override_cloud_descriptor
run cloud_requires_operator_bfs_password
run cloud_requires_db_password
run resolves_cloud_defaults_when_connection_fields_absent
run cloud_leaves_scope_untouched

if [[ "$FAILED" -ne 0 ]]; then
  printf '\nFAILED\n'
  exit 1
fi
printf '\nAll assertions passed.\n'
