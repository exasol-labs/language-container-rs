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

resolves_local_connection_from_descriptor() {
  local dir rc

  dir="$(mktemp -d)"
  printf '{"backend":"local","connection":{"host":"descriptor.example","sshPort":52341,"dbPort":52164}}\n' \
    >"$dir/deployment.json"
  printf '{"dbPassword":"secret"}\n' >"$dir/secrets.json"

  reset_connection_globals
  PORT="unresolved"
  resolve_deployment_connection "$dir" "$PERSONAL_DB_HOST_DEFAULT" >/dev/null 2>&1
  rc=$?

  check "resolve_deployment_connection succeeds given a complete local descriptor" "0" "$rc"
  check "HOST resolves from connection.host" "descriptor.example" "$HOST"
  check "PORT resolves from connection.dbPort" "52164" "$PORT"
  check "PASSWORD resolves from secrets.json .dbPassword" "secret" "$PASSWORD"

  PORT="unresolved"
  printf '{"backend":"local","connection":{"host":"descriptor.example","sshPort":52341,"dbPort":59446}}\n' \
    >"$dir/deployment.json"
  resolve_deployment_connection "$dir" "$PERSONAL_DB_HOST_DEFAULT" >/dev/null 2>&1
  check "a reassigned dbPort is picked up on the next read, never cached" "59446" "$PORT"

  # With no descriptor there is no .connection.host to read and no --host
  # supplied, so the empty-host check is what fails the resolution: a HOST left
  # over from the assertions above would make this resolve return 0.
  HOST=""
  rm -f "$dir/deployment.json"
  resolve_deployment_connection "$dir" "$PERSONAL_DB_HOST_DEFAULT" >/dev/null 2>&1
  check "a missing descriptor fails" "1" "$?"

  rm -rf "$dir"
}

unreadable_descriptor_fails_even_with_cli_overrides() {
  local dir rc

  # An empty deployment directory: every descriptor read fails. --host and
  # --password disarm both presence checks, so nothing downstream is left to
  # notice the failed reads — the resolver itself has to.
  dir="$(mktemp -d)"

  reset_connection_globals
  HOST="cloud.example"
  PASSWORD="pw"
  resolve_deployment_connection "$dir" '' >/dev/null 2>&1
  rc=$?

  check "an unreadable descriptor fails even when --host and --password are supplied" "1" "$rc"
  check "PORT is not left empty by a failed descriptor read" "8563" "$PORT"

  rm -rf "$dir"
}

cli_port_overrides_local_descriptor() {
  local dir

  dir="$(mktemp -d)"
  printf '{"backend":"local","connection":{"host":"127.0.0.1","sshPort":52341,"dbPort":52164}}\n' \
    >"$dir/deployment.json"
  printf '{"dbPassword":"secret"}\n' >"$dir/secrets.json"

  # Simulates an operator who passed --port: CLI_PORT is pre-set exactly as
  # main's arg loop would set it.
  reset_connection_globals
  PORT="unresolved"
  CLI_PORT="1234"
  resolve_deployment_connection "$dir" "$PERSONAL_DB_HOST_DEFAULT" >/dev/null 2>&1

  check "an explicit --port wins over connection.dbPort" "1234" "$PORT"

  rm -rf "$dir"
}

resolves_local_defaults_when_db_port_absent() {
  local dir rc

  dir="$(mktemp -d)"
  printf '{"backend":"local","connection":{"host":"127.0.0.1","sshPort":52341}}\n' \
    >"$dir/deployment.json"
  printf '{"dbPassword":"secret"}\n' >"$dir/secrets.json"

  reset_connection_globals
  PORT="unresolved"
  resolve_deployment_connection "$dir" "$PERSONAL_DB_HOST_DEFAULT" >/dev/null 2>&1
  rc=$?

  check "resolve_deployment_connection succeeds with dbPort absent" "0" "$rc"
  check "PORT falls back to 8563 when connection.dbPort is absent" "8563" "$PORT"

  rm -rf "$dir"
}

cli_host_overrides_local_descriptor() {
  local dir

  dir="$(mktemp -d)"
  printf '{"backend":"local","connection":{"host":"descriptor.example","sshPort":52341,"dbPort":52164}}\n' \
    >"$dir/deployment.json"
  printf '{"dbPassword":"secret"}\n' >"$dir/secrets.json"

  # Simulates an operator who passed --host: HOST is pre-set exactly as main's
  # arg loop would set it. Overriding the descriptor value (not the default)
  # pins the top of the precedence chain: the descriptor already beats the
  # default, so this is the only case that proves --host beats both.
  reset_connection_globals
  HOST="override.example"
  resolve_deployment_connection "$dir" "$PERSONAL_DB_HOST_DEFAULT" >/dev/null 2>&1

  check "an explicit --host wins over connection.host" "override.example" "$HOST"

  rm -rf "$dir"
}

resolves_local_host_default_when_absent() {
  local dir rc

  dir="$(mktemp -d)"
  printf '{"backend":"local","connection":{"sshPort":52341,"dbPort":52164}}\n' \
    >"$dir/deployment.json"
  printf '{"dbPassword":"secret"}\n' >"$dir/secrets.json"

  # No .connection.host and no --host: reset_connection_globals already leaves
  # HOST="", which is how "no --host" is expressed, so no sentinel is needed.
  reset_connection_globals
  resolve_deployment_connection "$dir" "$PERSONAL_DB_HOST_DEFAULT" >/dev/null 2>&1
  rc=$?

  check "resolve_deployment_connection succeeds with connection.host absent" "0" "$rc"
  check "HOST falls back to 127.0.0.1 when connection.host is absent" "127.0.0.1" "$HOST"

  rm -rf "$dir"
}

resolves_local_user_from_descriptor() {
  local dir

  dir="$(mktemp -d)"
  printf '{"backend":"local","connection":{"host":"127.0.0.1","sshPort":52341,"dbPort":52164,"username":"dbadmin"}}\n' \
    >"$dir/deployment.json"
  printf '{"dbPassword":"secret"}\n' >"$dir/secrets.json"

  # username must not be "sys": reset_connection_globals already writes
  # USER=sys, so a "sys" fixture would assert nothing.
  reset_connection_globals
  resolve_deployment_connection "$dir" "$PERSONAL_DB_HOST_DEFAULT" >/dev/null 2>&1

  check "USER resolves from connection.username" "dbadmin" "$USER"

  rm -rf "$dir"
}

parses_current_script_languages_from_query_output() {
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

refuses_to_register_when_the_current_value_cannot_be_read() {
  local stub_dir saved_path

  stub_dir="$(mktemp -d)"
  printf '#!/bin/sh\necho "exapump: could not connect to host" >&2\nexit 1\n' \
    >"$stub_dir/exapump"
  chmod +x "$stub_dir/exapump"
  saved_path="$PATH"
  PATH="$stub_dir:$PATH"
  current_script_languages "exasol://sys:x@127.0.0.1:8563" >/dev/null 2>&1
  check "a failed query fails the read instead of yielding an empty value" "1" "$?"
  PATH="$saved_path"
  rm -rf "$stub_dir"

  parse_script_languages "" >/dev/null 2>&1
  check "an empty query result fails the read" "1" "$?"

  parse_script_languages "CURRENT_SCRIPT_LANGUAGES" >/dev/null 2>&1
  check "a header-only query result fails the read" "1" "$?"
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
  PORT="unresolved"
  USER="unresolved"
  resolve_deployment_connection "$dir" '' >/dev/null 2>&1
  rc=$?

  check "resolve_deployment_connection succeeds given a complete cloud descriptor" "0" "$rc"
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
  resolve_deployment_connection "$dir" '' >/dev/null 2>&1
  rc=$?

  check "resolve_deployment_connection succeeds with CLI overrides" "0" "$rc"
  check "an explicit --host wins over connection.host" "override.example" "$HOST"
  check "an explicit --port wins over connection.dbPort" "1234" "$PORT"
  check "an explicit --user wins over connection.username" "admin" "$USER"
  check "an explicit --password wins over secrets.json .dbPassword" "overridepw" "$PASSWORD"

  rm -rf "$dir"
}

cloud_requires_operator_bfs_password() {
  local err rc names_bfs_password

  # A BucketFS credential is not a connection field: require_cloud_bfs_password
  # reads only BFS_PASSWORD, so no deployment fixture takes part.
  reset_connection_globals
  err="$(require_cloud_bfs_password 2>&1 >/dev/null)"
  rc=$?

  check "an empty --bfs-password fails the cloud BucketFS-password requirement" "1" "$rc"
  if [[ "$err" == *"--bfs-password"* ]]; then names_bfs_password=1; else names_bfs_password=0; fi
  check "the error names --bfs-password as the missing BucketFS credential" "1" "$names_bfs_password"

  BFS_PASSWORD="bfspw"
  require_cloud_bfs_password >/dev/null 2>&1
  check "a supplied --bfs-password lets the cloud install proceed" "0" "$?"
}

cloud_requires_host_when_descriptor_omits_it() {
  local dir rc

  dir="$(mktemp -d)"
  # No .connection.host: cloud is the one backend with no host default, so the
  # empty default_host the cloud call site passes must leave HOST unresolved and
  # fail the resolution. Every other cloud test supplies a descriptor host, which
  # wins before the default is ever consulted.
  printf '{"backend":"aws","connection":{"dbPort":8563,"username":"sys"}}\n' \
    >"$dir/deployment.json"
  printf '{"dbPassword":"secret"}\n' >"$dir/secrets.json"

  reset_connection_globals
  resolve_deployment_connection "$dir" '' >/dev/null 2>&1
  rc=$?

  check "cloud resolution fails when connection.host is absent and no --host is given" "1" "$rc"
  check "HOST stays empty because the cloud call site passes no host default" "" "$HOST"

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
  resolve_deployment_connection "$dir" '' >/dev/null 2>&1
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
  PORT="unresolved"
  USER="unresolved"
  resolve_deployment_connection "$dir" '' >/dev/null 2>&1
  rc=$?

  check "resolve_deployment_connection succeeds with dbPort/username absent" "0" "$rc"
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
  resolve_deployment_connection "$dir" '' >/dev/null 2>&1

  # Unlike the local transport, which forces SCOPE=SYSTEM, cloud resolution
  # MUST NOT touch SCOPE: cloud honors --scope, default SESSION.
  check "cloud resolution leaves SCOPE at its default (MUST NOT force SYSTEM)" \
    "SESSION" "$SCOPE"

  rm -rf "$dir"
}

# ── shared-directory mechanism fixtures ───────────────────────────────────────
write_bucketfs_mapping() { # write_bucketfs_mapping <dir> [<vm-path> <service> <bucket>]
  local dir="$1" vm_path="${2:-/exa/bucketfs/bfsdefault/default}"
  local service="${3:-bfsdefault}" bucket="${4:-default}"

  mkdir -p "$dir/local/runtime/vm-shared/exa"
  printf '%s\n%s\n' \
    '/exa/slc __builtin__ slc /exa/slc dDE= P' \
    "$vm_path $service $bucket /buckets/$service/$bucket - P" \
    >"$dir/local/runtime/vm-shared/exa/bucketfs.conf"
}

exists() {
  if [[ -e "$1" ]]; then printf 'present\n'; else printf 'absent\n'; fi
}

executable() {
  if [[ -x "$1" ]]; then printf 'executable\n'; else printf 'not executable\n'; fi
}

reset_bucketfs_globals() {
  BFS_SERVICE=bfsdefault
  BUCKET=default
  SLC_NAME=rustslc
}

rejects_path_unsafe_name_components() {
  local value err names_flag

  for value in "" "slc/rustslc" "." ".." "-rustslc"; do
    require_path_segment --slc-name "$value" >/dev/null 2>&1
    check "--slc-name [$value] is rejected as a path component" "1" "$?"
  done

  require_path_segment --slc-name rustslc >/dev/null 2>&1
  check "a plain path segment is accepted" "0" "$?"

  err="$(require_path_segment --bfs-service "../etc" 2>&1 >/dev/null)"
  if [[ "$err" == *"--bfs-service"* ]]; then names_flag=1; else names_flag=0; fi
  check "the rejection names the flag that carried the bad value" "1" "$names_flag"
}

resolves_shared_bucketfs_dir_from_mapping() {
  local dir outside err rc names_it

  dir="$(mktemp -d)"
  write_bucketfs_mapping "$dir"
  mkdir -p "$dir/local/runtime/vm-shared/exa/bucketfs/bfsdefault/default"

  check "the mapping line for the pair resolves its shared host directory" \
    "$dir/local/runtime/vm-shared/exa/bucketfs/bfsdefault/default" \
    "$(deployment_bucketfs_dir "$dir" bfsdefault default)"

  printf '/exa/slc __builtin__ slc /exa/slc dDE= P\n/exa/bucketfs/bfsdefault/default bfsdefault default /buckets/bfsdefault/default - P' \
    >"$dir/local/runtime/vm-shared/exa/bucketfs.conf"
  check "an unterminated last mapping line is still read" \
    "$dir/local/runtime/vm-shared/exa/bucketfs/bfsdefault/default" \
    "$(deployment_bucketfs_dir "$dir" bfsdefault default)"
  write_bucketfs_mapping "$dir"

  err="$(deployment_bucketfs_dir "$dir" bfsdefault nosuchbucket 2>&1 >/dev/null)"
  rc=$?
  check "a pair no mapping line names fails with the unserved-pair status" "3" "$rc"
  if [[ "$err" == *"bfsdefault/nosuchbucket"* && "$err" == *"bfsdefault/default"* ]]; then
    names_it=1
  else
    names_it=0
  fi
  check "the unserved-pair error names the requested pair and the served pairs" "1" "$names_it"

  rm -f "$dir/local/runtime/vm-shared/exa/bucketfs.conf"
  deployment_bucketfs_dir "$dir" bfsdefault default >/dev/null 2>&1
  check "a deployment with no mapping file fails with the absent-mapping status" "2" "$?"

  write_bucketfs_mapping "$dir" /exa/bucketfs/bfsdefault/uncreated bfsdefault uncreated
  err="$(deployment_bucketfs_dir "$dir" bfsdefault uncreated 2>&1 >/dev/null)"
  rc=$?
  check "a mapping line whose host directory does not exist fails" "1" "$rc"
  if [[ "$err" == *"$dir/local/runtime/vm-shared/exa/bucketfs/bfsdefault/uncreated"* ]]; then
    names_it=1
  else
    names_it=0
  fi
  check "the absent-directory error names the resolved host directory" "1" "$names_it"

  outside="$(mktemp -d)"
  mkdir -p "$outside/escaped"
  write_bucketfs_mapping "$dir" /exa/bucketfs/bfsdefault/escape bfsdefault escape
  mkdir -p "$dir/local/runtime/vm-shared/exa/bucketfs/bfsdefault"
  ln -s "$outside/escaped" "$dir/local/runtime/vm-shared/exa/bucketfs/bfsdefault/escape"
  deployment_bucketfs_dir "$dir" bfsdefault escape >/dev/null 2>&1
  check "a mapping line resolving outside the deployment directory fails" "1" "$?"

  rm -rf "$dir" "$outside"
}

extracts_slc_into_shared_bucketfs() {
  local dir bucket stage tarball broken

  reset_bucketfs_globals
  dir="$(mktemp -d)"
  write_bucketfs_mapping "$dir"
  bucket="$dir/local/runtime/vm-shared/exa/bucketfs/bfsdefault/default"
  mkdir -p "$bucket/rustslc" "$bucket/udf"
  : >"$bucket/rustslc/stale-from-an-earlier-install"
  : >"$bucket/udf/libother.so"

  stage="$(mktemp -d)"
  mkdir -p "$stage/exaudf"
  printf '#!/bin/sh\n' >"$stage/exaudf/exaudfclient"
  chmod 755 "$stage/exaudf/exaudfclient"
  tarball="$stage.tar.gz"
  tar -czf "$tarball" -C "$stage" .

  extract_slc_into_shared_bucketfs "$dir" "$tarball" >/dev/null 2>&1
  check "extracting into the shared bucket directory succeeds" "0" "$?"
  check "the stale file from the earlier install is gone" \
    "absent" "$(exists "$bucket/rustslc/stale-from-an-earlier-install")"
  check "exaudfclient lands executable" \
    "executable" "$(executable "$bucket/rustslc/exaudf/exaudfclient")"
  check "the operator's own artifact outside the SLC tree survives" \
    "present" "$(exists "$bucket/udf/libother.so")"

  extract_slc_into_shared_bucketfs "$dir" "$tarball" >/dev/null 2>&1
  check "re-running the extraction succeeds" "0" "$?"
  check "the SLC tree is there exactly once after the second extraction" \
    "1" "$(find "$bucket" -maxdepth 1 -name rustslc | wc -l | tr -d ' ')"
  check "the operator's own artifact survives the second extraction" \
    "present" "$(exists "$bucket/udf/libother.so")"

  chmod 644 "$stage/exaudf/exaudfclient"
  broken="$stage-broken.tar.gz"
  tar -czf "$broken" -C "$stage" .
  extract_slc_into_shared_bucketfs "$dir" "$broken" >/dev/null 2>&1
  check "a tarball whose exaudfclient is not executable fails the call" "1" "$?"

  SLC_NAME="../escape"
  extract_slc_into_shared_bucketfs "$dir" "$tarball" >/dev/null 2>&1
  check "an slc-name that is not a direct child of the bucket directory fails" "1" "$?"
  check "the refusal removes nothing outside the SLC tree" \
    "present" "$(exists "$bucket/udf/libother.so")"
  check "the refusal leaves the bucket directory itself in place" \
    "present" "$(exists "$bucket")"

  reset_bucketfs_globals
  rm -rf "$dir" "$stage" "$tarball" "$broken"
}

selects_local_mechanism_from_deployment_directory() {
  local dir err rc names_it

  dir="$(mktemp -d)"
  printf '{"backend":"local","connection":{"host":"127.0.0.1","sshPort":52341,"dbPort":52164}}\n' \
    >"$dir/deployment.json"
  mkdir -p "$dir/local"
  : >"$dir/local/node_access.pem"
  write_bucketfs_mapping "$dir"
  mkdir -p "$dir/local/runtime/vm-shared/exa/bucketfs/bfsdefault/default"

  check "a deployment publishing both SSH inputs selects the SSH mechanism" \
    "ssh" "$(personal_local_mechanism "$dir" bfsdefault default)"

  printf '{"backend":"local","connection":{"host":"127.0.0.1","dbPort":52164}}\n' \
    >"$dir/deployment.json"
  check "a deployment publishing no SSH port selects the shared-directory mechanism" \
    "shared" "$(personal_local_mechanism "$dir" bfsdefault default)"

  printf '{"backend":"local","connection":{"host":"127.0.0.1","sshPort":52341,"dbPort":52164}}\n' \
    >"$dir/deployment.json"
  rm -f "$dir/local/node_access.pem"
  check "a deployment publishing no node key selects the shared-directory mechanism" \
    "shared" "$(personal_local_mechanism "$dir" bfsdefault default)"

  err="$(personal_local_mechanism "$dir" bfsdefault nosuchbucket 2>&1 >/dev/null)"
  rc=$?
  check "a pair the mapping does not serve fails the mechanism selection" "1" "$rc"
  if [[ "$err" == *"bfsdefault/nosuchbucket"* && "$err" == *"bfsdefault/default"* ]]; then
    names_it=1
  else
    names_it=0
  fi
  check "that failure names the requested pair and the pairs the mapping serves" "1" "$names_it"

  write_bucketfs_mapping "$dir" /exa/bucketfs/bfsdefault/uncreated bfsdefault uncreated
  err="$(personal_local_mechanism "$dir" bfsdefault uncreated 2>&1 >/dev/null)"
  rc=$?
  check "a mapped pair whose host directory is absent fails the selection" "1" "$rc"
  if [[ "$err" == *"vm-shared/exa/bucketfs/bfsdefault/uncreated"* && "$err" != *"--bucket"* ]]; then
    names_it=1
  else
    names_it=0
  fi
  check "that failure names the resolved host directory, not the flags" "1" "$names_it"

  rm -f "$dir/local/runtime/vm-shared/exa/bucketfs.conf"
  err="$(personal_local_mechanism "$dir" bfsdefault default 2>&1 >/dev/null)"
  rc=$?
  check "a deployment publishing neither mechanism's inputs fails" "1" "$rc"
  if [[ "$err" == *"sshPort"* && "$err" == *"node_access.pem"* && "$err" == *"bucketfs.conf"* ]]; then
    names_it=1
  else
    names_it=0
  fi
  check "that failure names both mechanisms' prerequisites" "1" "$names_it"

  rm -rf "$dir"
}

local_mechanisms_share_the_registration_inputs() {
  local ssh_dir shared_dir ssh_endpoint shared_endpoint entry
  local bucket_path ssh_dest shared_dest lands_at_bucket_path names_bucket_path

  reset_bucketfs_globals
  ssh_dir="$(mktemp -d)"
  printf '{"backend":"local","connection":{"host":"127.0.0.1","sshPort":52341,"dbPort":52164,"username":"dbadmin"}}\n' \
    >"$ssh_dir/deployment.json"
  printf '{"dbPassword":"secret"}\n' >"$ssh_dir/secrets.json"
  mkdir -p "$ssh_dir/local"
  : >"$ssh_dir/local/node_access.pem"

  shared_dir="$(mktemp -d)"
  printf '{"backend":"local","connection":{"host":"127.0.0.1","dbPort":52164,"username":"dbadmin"}}\n' \
    >"$shared_dir/deployment.json"
  printf '{"dbPassword":"secret"}\n' >"$shared_dir/secrets.json"
  write_bucketfs_mapping "$shared_dir"
  mkdir -p "$shared_dir/local/runtime/vm-shared/exa/bucketfs/bfsdefault/default"

  check "the SSH fixture selects the SSH mechanism" \
    "ssh" "$(personal_local_mechanism "$ssh_dir" bfsdefault default)"
  check "the shared-directory fixture selects the shared-directory mechanism" \
    "shared" "$(personal_local_mechanism "$shared_dir" bfsdefault default)"

  bucket_path="$BFS_SERVICE/$BUCKET/$SLC_NAME"
  ssh_dest="$VM_BUCKETFS_ROOT/$BFS_SERVICE/$BUCKET/$SLC_NAME"
  shared_dest="$(deployment_bucketfs_dir "$shared_dir" "$BFS_SERVICE" "$BUCKET")/$SLC_NAME"
  if [[ "$ssh_dest" == */"$bucket_path" ]]; then lands_at_bucket_path=1; else lands_at_bucket_path=0; fi
  check "the SSH mechanism places the SLC at the bucket path" "1" "$lands_at_bucket_path"
  if [[ "$shared_dest" == */"$bucket_path" ]]; then lands_at_bucket_path=1; else lands_at_bucket_path=0; fi
  check "the shared-directory mechanism places it at the same bucket path" "1" "$lands_at_bucket_path"

  entry="$(script_languages_entry "$BFS_SERVICE" "$BUCKET" "$SLC_NAME")"

  reset_connection_globals
  resolve_deployment_connection "$ssh_dir" "$PERSONAL_DB_HOST_DEFAULT" >/dev/null 2>&1
  ssh_endpoint="$USER@$HOST:$PORT"
  reset_connection_globals
  resolve_deployment_connection "$shared_dir" "$PERSONAL_DB_HOST_DEFAULT" >/dev/null 2>&1
  shared_endpoint="$USER@$HOST:$PORT"

  check "both mechanisms resolve the same endpoint" "$ssh_endpoint" "$shared_endpoint"
  check "that entry is the Personal bucket-root entry" "$PERSONAL_ENTRY" "$entry"
  if [[ "$entry" == *"#buckets/$bucket_path/exaudf/exaudfclient" ]]; then
    names_bucket_path=1
  else
    names_bucket_path=0
  fi
  check "the entry's fragment names the exaudfclient under that same bucket path" \
    "1" "$names_bucket_path"

  rm -rf "$ssh_dir" "$shared_dir"
}

run() {
  printf '%s\n' "$1"
  "$1"
}

run fragment_points_at_executable_no_leading_slash
run preserves_existing_script_languages
run reads_ssh_port_from_deployment_json
run resolves_local_connection_from_descriptor
run unreadable_descriptor_fails_even_with_cli_overrides
run cli_port_overrides_local_descriptor
run resolves_local_defaults_when_db_port_absent
run cli_host_overrides_local_descriptor
run resolves_local_host_default_when_absent
run resolves_local_user_from_descriptor
run parses_current_script_languages_from_query_output
run refuses_to_register_when_the_current_value_cannot_be_read
run selects_transport_from_backend
run resolves_cloud_connection_from_descriptor
run cli_flags_override_cloud_descriptor
run cloud_requires_operator_bfs_password
run cloud_requires_host_when_descriptor_omits_it
run cloud_requires_db_password
run resolves_cloud_defaults_when_connection_fields_absent
run cloud_leaves_scope_untouched
run rejects_path_unsafe_name_components
run resolves_shared_bucketfs_dir_from_mapping
run extracts_slc_into_shared_bucketfs
run selects_local_mechanism_from_deployment_directory
run local_mechanisms_share_the_registration_inputs

if [[ "$FAILED" -ne 0 ]]; then
  printf '\nFAILED\n'
  exit 1
fi
printf '\nAll assertions passed.\n'
