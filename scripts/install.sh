#!/usr/bin/env bash
# Build the SLC tarball, get it into BucketFS, and register the RUST script
# language in an Exasol instance.
#
# Two transports, selected by whether --deployment is given:
#
#   * Default (a normal cluster / docker-db / SaaS): upload the tarball over the
#     BucketFS HTTP API and register with ALTER SESSION|SYSTEM.
#   * --deployment NAME (Exasol Personal): serves BOTH local and cloud Personal
#     backends, discriminated at runtime by the deployment directory's
#     deployment.json .backend field:
#       - "local": Personal publishes only the SQL port from its VM and exposes
#         no BucketFS HTTP endpoint, so the tarball travels over SSH and is
#         extracted straight into the VM's BucketFS directory; the engine
#         reconciles a real bucket from it. The SSH port is reassigned on every
#         `exasol start`, so it is read from the deployment descriptor on every
#         run and never cached. Registration uses ALTER SYSTEM and preserves
#         every pre-existing SCRIPT_LANGUAGES entry, so re-running after
#         `exasol stop && exasol start` picks up the reassigned port itself.
#       - any other backend (cloud, e.g. aws/azure/exoscale/stackit): reaches
#         the DB over the network and exposes the ordinary BucketFS HTTP
#         endpoint, so it falls through to the normal HTTP transport above,
#         with host/port/user/DB-password resolved from deployment.json and
#         secrets.json in the deployment directory instead of typed by hand.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
# shellcheck source=lib/script_languages.sh
source "$SCRIPT_DIR/lib/script_languages.sh"

# ── Personal-transport constants ──────────────────────────────────────────────
DEPLOYMENT_ROOT="$HOME/.exasol/personal/deployments"
DEPLOYMENT_DESCRIPTOR="deployment.json"
SECRETS_DESCRIPTOR="secrets.json"
NODE_KEY_RELATIVE_PATH="local/node_access.pem"
VM_BUCKETFS_ROOT="/var/lib/exa/bucketfs"
SCRIPT_LANGUAGES_COLUMN="CURRENT_SCRIPT_LANGUAGES"
RECONCILE_SECONDS=3
PERSONAL_DB_HOST=127.0.0.1
PERSONAL_DB_PORT=8563
SSH_HOST=127.0.0.1
SSH_OPTIONS=(-o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
  -o IdentitiesOnly=yes -o LogLevel=ERROR)

# ── defaults ──────────────────────────────────────────────────────────────────
HOST=""
PORT=8563
USER=sys
# Set only when the operator passes --port/--user, so the cloud branch can tell
# an explicit override from the PORT=8563/USER=sys defaults above.
CLI_PORT=""
CLI_USER=""
PASSWORD=""
BFS_PORT=2581
BFS_PASSWORD=""
BUCKET=default
BFS_SERVICE=bfsdefault
SLC_NAME=rustslc
SCOPE=SESSION
SKIP_BUILD=0
DEPLOYMENT=""
SSH_USER=root
TMP_DIR=""

# ── usage ─────────────────────────────────────────────────────────────────────
usage() {
  cat <<EOF
Usage: $(basename "$0") [OPTIONS]

Build the language container image, get it into BucketFS, and register the
RUST script language in Exasol.

Transport is chosen by --deployment:
  * without it — upload over the BucketFS HTTP API (normal cluster / SaaS);
  * with it    — Exasol Personal, local OR cloud, discriminated at runtime by
                 the deployment directory's deployment.json .backend field:
                   - "local": copy over SSH into the deployment's VM.
                   - any other backend (cloud): resolve host/port/user/DB
                     password from the deployment directory and use the same
                     HTTP transport as the default (no --deployment) path.

Required (HTTP transport, the default):
  -H, --host HOST            Exasol host
  -p, --password PASS        Exasol DB password
  -w, --bfs-password PASS    BucketFS write password

Required (Personal transport, local backend):
  -D, --deployment NAME      Personal deployment name (a directory under
                             $DEPLOYMENT_ROOT); SQL host/port are fixed at
                             ${PERSONAL_DB_HOST}:${PERSONAL_DB_PORT} and no BucketFS password is needed
  -p, --password PASS        Exasol DB password

Required (Personal transport, cloud backend):
  -D, --deployment NAME      Personal deployment name (a directory under
                             $DEPLOYMENT_ROOT); host/port/user/DB password are
                             read from deployment.json/secrets.json in it
  -w, --bfs-password PASS    BucketFS write password (REQUIRED — cloud
                             Personal provisions none; NOT needed for local)

Options:
  -P, --port PORT            Exasol DB port          (default: 8563; ignored for local --deployment, honored for cloud --deployment)
  -u, --user USER            Exasol user             (default: sys; honored for --deployment on both backends)
      --bfs-port PORT        BucketFS HTTPS port     (default: 2581; HTTP transport only)
      --bucket NAME          BucketFS bucket         (default: default)
      --bfs-service NAME     BucketFS service name   (default: bfsdefault)
      --slc-name NAME        SLC name in BucketFS    (default: rustslc)
      --scope SESSION|SYSTEM ALTER scope             (default: SESSION; local --deployment forces SYSTEM, cloud --deployment honors --scope)
      --ssh-user USER        VM SSH user             (default: root; local Personal transport only)
      --skip-build           Skip docker build; use SLC_TARBALL if set
  -h, --help                 Show this help

Environment:
  SLC_TARBALL                Use this prebuilt tarball instead of running
                             \`docker build\` (implies --skip-build)

The default build path needs Docker plus cargo-about (with network access for
the GCC-exception fetch), used by dist/generate-licenses.sh; the local Personal
transport additionally needs jq and ssh/scp; the cloud Personal transport
additionally needs jq. exapump is always required.

Examples:
  # Docker-db with default credentials:
  $(basename "$0") --host localhost --password exasol --bfs-password secret

  # SaaS / enterprise, persist across sessions:
  $(basename "$0") --host my.exasol.cloud --user admin --password s3cr3t \\
    --bfs-password bfspass --scope SYSTEM

  # Exasol Personal, local backend:
  $(basename "$0") --deployment my-db --password exasol

  # Exasol Personal, cloud backend:
  $(basename "$0") --deployment my-cloud-db --bfs-password bfspass
EOF
}

die() {
  echo "error: $*" >&2
  exit 1
}

require_command() {
  command -v "$1" >/dev/null 2>&1 || die "$1 is required but was not found on PATH"
}

# ── Personal: deployment descriptor (read fresh on every run) ─────────────────
deployment_ssh_port() {
  local descriptor="$1/$DEPLOYMENT_DESCRIPTOR" port

  if [[ ! -r "$descriptor" ]]; then
    echo "error: no readable deployment descriptor at $descriptor" >&2
    return 1
  fi

  port="$(jq -r '.connection.sshPort // empty' "$descriptor")"
  if [[ ! "$port" =~ ^[0-9]+$ ]]; then
    echo "error: $descriptor carries no numeric connection.sshPort" >&2
    return 1
  fi

  printf '%s\n' "$port"
}

deployment_key_path() {
  printf '%s\n' "$1/$NODE_KEY_RELATIVE_PATH"
}

# ── Cloud: deployment descriptor field access ─────────────────────────────────
deployment_backend() {
  local descriptor="$1/$DEPLOYMENT_DESCRIPTOR" backend

  if [[ ! -r "$descriptor" ]]; then
    echo "error: no readable deployment descriptor at $descriptor" >&2
    return 1
  fi

  backend="$(jq -r '.backend // empty' "$descriptor")"
  if [[ -z "$backend" ]]; then
    echo "error: $descriptor carries no .backend field" >&2
    return 1
  fi

  printf '%s\n' "$backend"
}

# Prints a deployment.json field selected by a jq path, or the given default
# when the field is absent or empty. The sole reader for every .connection.*
# field so a new one costs a call site, not a new function.
deployment_field() {
  local dir="$1" jq_path="$2" default="$3"
  local descriptor="$dir/$DEPLOYMENT_DESCRIPTOR" value

  if [[ ! -r "$descriptor" ]]; then
    echo "error: no readable deployment descriptor at $descriptor" >&2
    return 1
  fi

  value="$(jq -r "${jq_path} // empty" "$descriptor")"
  if [[ -z "$value" ]]; then
    value="$default"
  fi

  printf '%s\n' "$value"
}

deployment_db_password() {
  local descriptor="$1/$SECRETS_DESCRIPTOR" password

  if [[ ! -r "$descriptor" ]]; then
    echo "error: no readable secrets descriptor at $descriptor" >&2
    return 1
  fi

  password="$(jq -r '.dbPassword // empty' "$descriptor")"
  printf '%s\n' "$password"
}

# Finalize the normal-path connection globals HOST/PORT/USER/PASSWORD for a
# cloud Exasol Personal deployment from its deployment.json + secrets.json, with
# any explicit CLI flag winning over the descriptor. This is the single owner of
# the cloud-resolution decision: it also owns the resolved-DB-password presence
# check (an empty password after any --password override is the failure) and the
# --bfs-password requirement (Personal provisions no BucketFS password).
#
# Returns 1 (never exits) on a missing value so the sourced unit harness drives
# it in-process. main invokes it as `resolve_cloud_connection "$dir" || die …`,
# whose condition context suspends errexit inside so a failed accessor falls
# through to these presence checks instead of aborting.
resolve_cloud_connection() {
  local dir="$1"
  local descriptor_host descriptor_port descriptor_user
  local descriptor_password=""

  # Capture each accessor on its own line: a bare `local x="$(accessor)"` would
  # mask the command substitution's exit status behind the `local` builtin's.
  descriptor_host="$(deployment_field "$dir" '.connection.host' '')" || true
  descriptor_port="$(deployment_field "$dir" '.connection.dbPort' '8563')" || true
  descriptor_user="$(deployment_field "$dir" '.connection.username' 'sys')" || true

  [[ -z "$HOST" ]] && HOST="$descriptor_host"
  if [[ -n "$CLI_PORT" ]]; then PORT="$CLI_PORT"; else PORT="$descriptor_port"; fi
  if [[ -n "$CLI_USER" ]]; then USER="$CLI_USER"; else USER="$descriptor_user"; fi
  if [[ -z "$PASSWORD" ]]; then
    descriptor_password="$(deployment_db_password "$dir")" || true
    PASSWORD="$descriptor_password"
  fi

  if [[ -z "$HOST" ]]; then
    echo "error: no DB host for the cloud deployment; set .connection.host in $dir/$DEPLOYMENT_DESCRIPTOR or pass --host" >&2
    return 1
  fi
  if [[ -z "$PASSWORD" ]]; then
    echo "error: no DB password for the cloud deployment; set .dbPassword in $dir/$SECRETS_DESCRIPTOR or pass --password" >&2
    return 1
  fi
  if [[ -z "$BFS_PASSWORD" ]]; then
    echo "error: Exasol Personal provisions no BucketFS password; --bfs-password is required for a cloud deployment" >&2
    return 1
  fi

  return 0
}

# ── Personal: SCRIPT_LANGUAGES value handling ─────────────────────────────────
csv_unquote() {
  local field="$1"

  if [[ "$field" == '"'*'"' ]]; then
    field="${field:1:${#field}-2}"
    field="${field//\"\"/\"}"
  fi

  printf '%s\n' "$field"
}

# Read the current parameter value out of the CSV result of the query below.
# An unrecognized shape is an error rather than an empty value: silently
# treating it as empty would drop every language the database already has.
parse_script_languages() {
  local output="$1" candidate line="" value token
  local -a tokens

  while IFS= read -r candidate; do
    [[ -z "$candidate" ]] && continue
    [[ "$candidate" == "$SCRIPT_LANGUAGES_COLUMN" ]] && continue
    line="$candidate"
  done <<<"$output"

  value="$(csv_unquote "$line")"
  read -ra tokens <<<"$value"
  for token in ${tokens[@]+"${tokens[@]}"}; do
    if [[ "$token" != *=* ]]; then
      echo "error: unrecognized SCRIPT_LANGUAGES query output: $line" >&2
      return 1
    fi
  done

  printf '%s\n' "$value"
}

# ALTER SYSTEM replaces the whole parameter, so every pre-existing entry is
# carried over; a stale RUST entry is dropped so re-running stays idempotent.
script_languages_with_rust_entry() {
  local existing="$1" entry="$2" word
  local -a words kept

  kept=()
  read -ra words <<<"$existing"
  for word in ${words[@]+"${words[@]}"}; do
    case "$word" in
      "${SCRIPT_LANGUAGE_ALIAS}="*) continue ;;
    esac
    kept+=("$word")
  done
  kept+=("$entry")

  printf '%s\n' "${kept[*]}"
}

current_script_languages() {
  local dsn="$1" output

  output="$(exapump sql -f csv \
    "SELECT SYSTEM_VALUE AS ${SCRIPT_LANGUAGES_COLUMN} FROM EXA_PARAMETERS WHERE PARAMETER_NAME = 'SCRIPT_LANGUAGES'" \
    -d "$dsn")"
  parse_script_languages "$output"
}

# ── Personal: SSH copy + filesystem BucketFS reconciliation ───────────────────
extract_slc_into_bucketfs() {
  local key="$1" ssh_port="$2" tarball="$3"
  local remote="${SSH_USER}@${SSH_HOST}"
  local staged="/tmp/${SLC_NAME}.tar.gz"
  local dest="${VM_BUCKETFS_ROOT}/${BFS_SERVICE}/${BUCKET}/${SLC_NAME}"

  scp "${SSH_OPTIONS[@]}" -i "$key" -P "$ssh_port" "$tarball" "${remote}:${staged}"
  ssh "${SSH_OPTIONS[@]}" -i "$key" -p "$ssh_port" "$remote" \
    "set -e; rm -rf '${dest}'; mkdir -p '${dest}'; tar -xzf '${staged}' -C '${dest}'; rm -f '${staged}'; test -x '${dest}/exaudf/exaudfclient'"
}

# ── argument parsing ───────────────────────────────────────────────────────────
main() {
while [[ $# -gt 0 ]]; do
  case "$1" in
    -H|--host)           HOST="$2";        shift 2 ;;
    -P|--port)           PORT="$2"; CLI_PORT="$2"; shift 2 ;;
    -u|--user)           USER="$2"; CLI_USER="$2"; shift 2 ;;
    -p|--password)       PASSWORD="$2";    shift 2 ;;
       --bfs-port)       BFS_PORT="$2";    shift 2 ;;
    -w|--bfs-password)   BFS_PASSWORD="$2"; shift 2 ;;
       --bucket)         BUCKET="$2";      shift 2 ;;
       --bfs-service)    BFS_SERVICE="$2"; shift 2 ;;
       --slc-name)       SLC_NAME="$2";    shift 2 ;;
       --scope)          SCOPE="$2";       shift 2 ;;
    -D|--deployment)     DEPLOYMENT="$2";  shift 2 ;;
       --ssh-user)       SSH_USER="$2";    shift 2 ;;
       --skip-build)     SKIP_BUILD=1;     shift   ;;
    -h|--help)           usage; exit 0 ;;
    *) echo "Unknown option: $1" >&2; usage >&2; exit 1 ;;
  esac
done

# ── validation ─────────────────────────────────────────────────────────────────
# Interpolated into BucketFS paths (an `rm -rf` on the VM for Personal) — an
# empty component would widen the destination.
[[ -z "$BFS_SERVICE" ]] && die "--bfs-service must not be empty"
[[ -z "$BUCKET" ]]      && die "--bucket must not be empty"
[[ -z "$SLC_NAME" ]]    && die "--slc-name must not be empty"

# ── transport selection ────────────────────────────────────────────────────────
# --deployment routes on the descriptor's .backend: "local" keeps the Exasol
# Personal SSH + filesystem transport (forced 127.0.0.1:8563, ALTER SYSTEM); any
# cloud backend resolves the normal HTTP connection details from the deployment
# directory and falls through to the default upload-and-register path. Without
# --deployment the default path handles a normal cluster / SaaS.
LOCAL_TRANSPORT=0
DEPLOYMENT_DIR=""
SSH_PORT=""
NODE_KEY=""
if [[ -n "$DEPLOYMENT" ]]; then
  require_command jq
  require_command exapump
  DEPLOYMENT_DIR="$DEPLOYMENT_ROOT/$DEPLOYMENT"
  [[ -d "$DEPLOYMENT_DIR" ]] || die "no Personal deployment at $DEPLOYMENT_DIR"

  # Condition context: a return 1 from the accessor becomes a clean die rather
  # than an errexit abort.
  local backend
  backend="$(deployment_backend "$DEPLOYMENT_DIR")" \
    || die "cannot determine the deployment backend from $DEPLOYMENT_DIR/$DEPLOYMENT_DESCRIPTOR"

  if [[ "$backend" == "local" ]]; then
    # Personal-local always talks to the VM's local SQL port; ALTER SYSTEM so the
    # registration survives a restart. Fresh SSH port + node key on every run
    # (the port is reassigned on every `exasol start`).
    LOCAL_TRANSPORT=1
    HOST="$PERSONAL_DB_HOST"
    PORT="$PERSONAL_DB_PORT"
    SCOPE=SYSTEM
    [[ -z "$PASSWORD" ]] && die "--password is required"
    SSH_PORT="$(deployment_ssh_port "$DEPLOYMENT_DIR")"
    NODE_KEY="$(deployment_key_path "$DEPLOYMENT_DIR")"
    [[ -r "$NODE_KEY" ]] || die "no readable node key at $NODE_KEY"
  else
    # Cloud reaches the DB over the network and exposes the ordinary BucketFS
    # HTTP endpoint; --password is an optional override here, so the resolved-
    # password check lives inside resolve_cloud_connection, not before the branch.
    resolve_cloud_connection "$DEPLOYMENT_DIR" \
      || die "cannot resolve the cloud deployment connection from $DEPLOYMENT_DIR"
  fi
else
  require_command exapump
  [[ -z "$HOST" ]]         && die "--host is required"
  [[ -z "$PASSWORD" ]]     && die "--password is required"
  [[ -z "$BFS_PASSWORD" ]] && die "--bfs-password is required"
fi

SCOPE_UPPER="${SCOPE^^}"
if [[ "$SCOPE_UPPER" != "SESSION" && "$SCOPE_UPPER" != "SYSTEM" ]]; then
  die "--scope must be SESSION or SYSTEM"
fi

# ── step 1: build (shared) ─────────────────────────────────────────────────────
# A prebuilt SLC_TARBALL implies --skip-build.
[[ -n "${SLC_TARBALL:-}" ]] && SKIP_BUILD=1

if [[ "$SKIP_BUILD" -eq 0 ]]; then
  require_command docker
  require_command cargo-about  # used by dist/generate-licenses.sh below
  echo "==> Generating license bundles …"
  bash "$REPO_ROOT/dist/generate-licenses.sh"
  echo "==> Building SLC tarball for the host architecture (--target artifact) …"
  TMP_DIR=$(mktemp -d /tmp/slc-XXXXXX)
  trap 'rm -rf "$TMP_DIR"' EXIT
  docker build \
    -f "$REPO_ROOT/Dockerfile.alpine" \
    --target artifact \
    --output "type=local,dest=$TMP_DIR" \
    "$REPO_ROOT"
  TMP_TAR="$TMP_DIR/lc-rs.tar.gz"
  echo "==> Build complete."
else
  if [[ -z "${SLC_TARBALL:-}" ]]; then
    die "--skip-build requires SLC_TARBALL to be set"
  fi
  echo "==> Skipping build; using SLC_TARBALL=${SLC_TARBALL}."
  TMP_TAR="$SLC_TARBALL"
fi

# ── step 2: report tarball ─────────────────────────────────────────────────────
echo "==> Tarball ready: $TMP_TAR ($(du -sh "$TMP_TAR" | cut -f1))."

# ── step 3: transport + register ───────────────────────────────────────────────
if [[ "$LOCAL_TRANSPORT" -eq 1 ]]; then
  DSN="exasol://${USER}:${PASSWORD}@${HOST}:${PORT}?validateservercertificate=0"
  SLC_PATH="$SLC_NAME"

  echo "==> Copying the SLC into ${VM_BUCKETFS_ROOT}/${BFS_SERVICE}/${BUCKET}/${SLC_NAME} (ssh port ${SSH_PORT}) …"
  extract_slc_into_bucketfs "$NODE_KEY" "$SSH_PORT" "$TMP_TAR"
  echo "==> Waiting ${RECONCILE_SECONDS}s for the engine to reconcile the bucket …"
  sleep "$RECONCILE_SECONDS"

  ENTRY="$(script_languages_entry "$BFS_SERVICE" "$BUCKET" "$SLC_PATH")"
  EXISTING="$(current_script_languages "$DSN")"
  SCRIPT_LANGUAGES="$(script_languages_with_rust_entry "$EXISTING" "$ENTRY")"

  echo "==> Registering RUST (ALTER SYSTEM SET SCRIPT_LANGUAGES) …"
  exapump sql "ALTER SYSTEM SET SCRIPT_LANGUAGES='${SCRIPT_LANGUAGES}'" -d "$DSN"
  echo "==> Done. The RUST script language is now available at /buckets/${BFS_SERVICE}/${BUCKET}/${SLC_NAME}/."
  echo
  echo "    SCRIPT_LANGUAGES entry:"
  echo "    ${ENTRY}"
else
  BFS_PATH="slc/${SLC_NAME}.tar.gz"
  SLC_PATH="slc/${SLC_NAME}"

  echo "==> Uploading to BucketFS: ${BFS_SERVICE}/${BUCKET}/${BFS_PATH} …"
  exapump bucketfs cp "$TMP_TAR" "$BFS_PATH" \
    --bfs-host "$HOST" \
    --bfs-port "$BFS_PORT" \
    --bfs-bucket "$BUCKET" \
    --bfs-write-password "$BFS_PASSWORD" \
    --bfs-tls true \
    --bfs-validate-certificate false
  echo "==> Upload complete."

  SCRIPT_LANGUAGES="$(script_languages_entry "$BFS_SERVICE" "$BUCKET" "$SLC_PATH")"
  DSN="exasol://${USER}:${PASSWORD}@${HOST}:${PORT}?validateservercertificate=0"

  echo "==> Registering RUST language (ALTER ${SCOPE_UPPER} SET SCRIPT_LANGUAGES) …"
  exapump sql \
    "ALTER ${SCOPE_UPPER} SET SCRIPT_LANGUAGES='${SCRIPT_LANGUAGES}'" \
    -d "$DSN"
  echo "==> Done. The RUST script language is now available."
  echo
  echo "    SCRIPT_LANGUAGES entry:"
  echo "    ${SCRIPT_LANGUAGES}"
fi
}

# Only run when executed directly; sourcing (e.g. from the unit tests) defines
# the functions and constants above without kicking off an install.
if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
  main "$@"
fi
