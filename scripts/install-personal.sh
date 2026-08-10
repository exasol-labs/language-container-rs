#!/usr/bin/env bash
# Install the Rust SLC onto an Exasol Personal deployment and register the RUST
# script language.
#
# Personal publishes only the SQL port from its VM and exposes no BucketFS HTTP
# endpoint, so the tarball travels over SSH and is extracted straight into the
# VM's BucketFS directory; the engine reconciles a real bucket from it. The SSH
# port is reassigned on every `exasol start`, so it is read from the deployment
# descriptor on every run and never cached.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
# shellcheck source=lib/script_languages.sh
source "$SCRIPT_DIR/lib/script_languages.sh"

# ── constants ─────────────────────────────────────────────────────────────────
DEPLOYMENT_ROOT="$HOME/.exasol/personal/deployments"
DEPLOYMENT_DESCRIPTOR="deployment.json"
NODE_KEY_RELATIVE_PATH="local/node_access.pem"
VM_BUCKETFS_ROOT="/var/lib/exa/bucketfs"
SCRIPT_LANGUAGES_COLUMN="CURRENT_SCRIPT_LANGUAGES"
RECONCILE_SECONDS=3
DB_HOST=127.0.0.1
DB_PORT=8563
SSH_HOST=127.0.0.1
SSH_OPTIONS=(-o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
  -o IdentitiesOnly=yes -o LogLevel=ERROR)

# ── defaults ──────────────────────────────────────────────────────────────────
DEPLOYMENT=""
DB_USER=sys
DB_PASSWORD=""
BUCKET=default
BFS_SERVICE=bfsdefault
SLC_NAME=rustslc
SSH_USER=root
TMP_DIR=""

# ── usage ─────────────────────────────────────────────────────────────────────
usage() {
  cat <<EOF
Usage: $(basename "$0") [OPTIONS]

Build the language container, copy it into an Exasol Personal deployment's
BucketFS directory over SSH, and register the RUST script language.

Required:
  -D, --deployment NAME      Personal deployment name (a directory under
                             $DEPLOYMENT_ROOT)
  -p, --password PASS        Exasol DB password

Options:
  -u, --user USER            Exasol user             (default: sys)
      --bucket NAME          BucketFS bucket         (default: default)
      --bfs-service NAME     BucketFS service name   (default: bfsdefault)
      --slc-name NAME        SLC name in BucketFS    (default: rustslc)
      --ssh-user USER        VM SSH user             (default: root)
  -h, --help                 Show this help

Environment:
  SLC_TARBALL                Use this prebuilt tarball instead of running
                             \`docker build\`

Requires: jq, ssh/scp, exapump, and Docker unless SLC_TARBALL is set.

Registration uses ALTER SYSTEM so it survives a restart, and preserves every
pre-existing SCRIPT_LANGUAGES entry. Re-run the script after
\`exasol stop && exasol start\` — it picks up the reassigned SSH port itself.

Example:
  $(basename "$0") --deployment my-db --password exasol
EOF
}

die() {
  echo "error: $*" >&2
  exit 1
}

require_command() {
  command -v "$1" >/dev/null 2>&1 || die "$1 is required but was not found on PATH"
}

# ── deployment descriptor (read fresh on every run) ───────────────────────────
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

# ── SCRIPT_LANGUAGES value handling ───────────────────────────────────────────
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

register_script_languages() {
  local dsn="$1" value="$2"

  exapump sql "ALTER SYSTEM SET SCRIPT_LANGUAGES='${value}'" -d "$dsn"
}

# ── transport: SSH copy + filesystem BucketFS reconciliation ──────────────────
extract_slc_into_bucketfs() {
  local key="$1" ssh_port="$2" tarball="$3"
  local remote="${SSH_USER}@${SSH_HOST}"
  local staged="/tmp/${SLC_NAME}.tar.gz"
  local dest="${VM_BUCKETFS_ROOT}/${BFS_SERVICE}/${BUCKET}/${SLC_NAME}"

  scp "${SSH_OPTIONS[@]}" -i "$key" -P "$ssh_port" "$tarball" "${remote}:${staged}"
  ssh "${SSH_OPTIONS[@]}" -i "$key" -p "$ssh_port" "$remote" \
    "set -e; rm -rf '${dest}'; mkdir -p '${dest}'; tar -xzf '${staged}' -C '${dest}'; rm -f '${staged}'; test -x '${dest}/exaudf/exaudfclient'"
}

# ── argument parsing ──────────────────────────────────────────────────────────
parse_arguments() {
  while [[ $# -gt 0 ]]; do
    case "$1" in
      -D|--deployment)   DEPLOYMENT="$2";  shift 2 ;;
      -u|--user)         DB_USER="$2";     shift 2 ;;
      -p|--password)     DB_PASSWORD="$2"; shift 2 ;;
         --bucket)       BUCKET="$2";      shift 2 ;;
         --bfs-service)  BFS_SERVICE="$2"; shift 2 ;;
         --slc-name)     SLC_NAME="$2";    shift 2 ;;
         --ssh-user)     SSH_USER="$2";    shift 2 ;;
      -h|--help)         usage; exit 0 ;;
      *) echo "Unknown option: $1" >&2; usage >&2; exit 1 ;;
    esac
  done

  [[ -z "$DEPLOYMENT" ]]  && die "--deployment is required"
  [[ -z "$DB_PASSWORD" ]] && die "--password is required"
  # Interpolated into an `rm -rf` on the VM — an empty component would widen it.
  [[ -z "$BFS_SERVICE" ]] && die "--bfs-service must not be empty"
  [[ -z "$BUCKET" ]]      && die "--bucket must not be empty"
  [[ -z "$SLC_NAME" ]]    && die "--slc-name must not be empty"
  return 0
}

main() {
  parse_arguments "$@"
  require_command jq
  require_command exapump

  local deployment_dir ssh_port key tarball dsn entry existing merged
  deployment_dir="$DEPLOYMENT_ROOT/$DEPLOYMENT"
  [[ -d "$deployment_dir" ]] || die "no Personal deployment at $deployment_dir"

  ssh_port="$(deployment_ssh_port "$deployment_dir")"
  key="$(deployment_key_path "$deployment_dir")"
  [[ -r "$key" ]] || die "no readable node key at $key"

  if [[ -n "${SLC_TARBALL:-}" ]]; then
    echo "==> Using SLC_TARBALL=${SLC_TARBALL}."
    tarball="$SLC_TARBALL"
  else
    require_command docker
    echo "==> Generating license bundles …"
    bash "$REPO_ROOT/dist/generate-licenses.sh"
    echo "==> Building the SLC tarball for the host architecture …"
    TMP_DIR="$(mktemp -d /tmp/slc-XXXXXX)"
    trap 'rm -rf "$TMP_DIR"' EXIT
    docker build \
      -f "$REPO_ROOT/Dockerfile.alpine" \
      --target artifact \
      --output "type=local,dest=$TMP_DIR" \
      "$REPO_ROOT"
    tarball="$TMP_DIR/lc-rs.tar.gz"
  fi
  echo "==> Tarball ready: $tarball ($(du -sh "$tarball" | cut -f1))."

  echo "==> Copying the SLC into ${VM_BUCKETFS_ROOT}/${BFS_SERVICE}/${BUCKET}/${SLC_NAME} (ssh port ${ssh_port}) …"
  extract_slc_into_bucketfs "$key" "$ssh_port" "$tarball"
  echo "==> Waiting ${RECONCILE_SECONDS}s for the engine to reconcile the bucket …"
  sleep "$RECONCILE_SECONDS"

  dsn="exasol://${DB_USER}:${DB_PASSWORD}@${DB_HOST}:${DB_PORT}?validateservercertificate=0"
  entry="$(script_languages_entry "$BFS_SERVICE" "$BUCKET" "$SLC_NAME")"
  existing="$(current_script_languages "$dsn")"
  merged="$(script_languages_with_rust_entry "$existing" "$entry")"

  echo "==> Registering RUST (ALTER SYSTEM SET SCRIPT_LANGUAGES) …"
  register_script_languages "$dsn" "$merged"
  echo "==> Done. The RUST script language is now available at /buckets/${BFS_SERVICE}/${BUCKET}/${SLC_NAME}/."
  echo
  echo "    SCRIPT_LANGUAGES entry:"
  echo "    ${entry}"
}

if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
  main "$@"
fi
