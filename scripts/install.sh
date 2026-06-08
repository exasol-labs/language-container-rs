#!/usr/bin/env bash
# Build the SLC Docker image, export it to BucketFS, and register the RUST
# script language in an Exasol instance.
set -euo pipefail

# ── defaults ──────────────────────────────────────────────────────────────────
HOST=""
PORT=8563
USER=sys
PASSWORD=""
BFS_PORT=2581
BFS_PASSWORD=""
BUCKET=default
BFS_SERVICE=bfsdefault
SLC_NAME=rustslc
SCOPE=SESSION
SKIP_BUILD=0

# ── usage ─────────────────────────────────────────────────────────────────────
usage() {
  cat <<EOF
Usage: $(basename "$0") [OPTIONS]

Build the language container image, upload it to BucketFS, and register the
RUST script language in Exasol.

Required:
  -H, --host HOST            Exasol host
  -p, --password PASS        Exasol DB password
  -w, --bfs-password PASS    BucketFS write password

Options:
  -P, --port PORT            Exasol DB port          (default: 8563)
  -u, --user USER            Exasol user             (default: sys)
      --bfs-port PORT        BucketFS HTTPS port     (default: 2581)
      --bucket NAME          BucketFS bucket         (default: default)
      --bfs-service NAME     BucketFS service name   (default: bfsdefault)
      --slc-name NAME        SLC name in BucketFS    (default: rustslc)
      --scope SESSION|SYSTEM ALTER scope             (default: SESSION)
      --skip-build           Skip docker build; use existing slc-rs-slim:dev
  -h, --help                 Show this help

Examples:
  # Docker-db with default credentials:
  $(basename "$0") --host localhost --password exasol --bfs-password secret

  # SaaS / enterprise, persist across sessions:
  $(basename "$0") --host my.exasol.cloud --user admin --password s3cr3t \\
    --bfs-password bfspass --scope SYSTEM
EOF
}

# ── argument parsing ───────────────────────────────────────────────────────────
while [[ $# -gt 0 ]]; do
  case "$1" in
    -H|--host)           HOST="$2";        shift 2 ;;
    -P|--port)           PORT="$2";        shift 2 ;;
    -u|--user)           USER="$2";        shift 2 ;;
    -p|--password)       PASSWORD="$2";    shift 2 ;;
       --bfs-port)       BFS_PORT="$2";    shift 2 ;;
    -w|--bfs-password)   BFS_PASSWORD="$2"; shift 2 ;;
       --bucket)         BUCKET="$2";      shift 2 ;;
       --bfs-service)    BFS_SERVICE="$2"; shift 2 ;;
       --slc-name)       SLC_NAME="$2";    shift 2 ;;
       --scope)          SCOPE="$2";       shift 2 ;;
       --skip-build)     SKIP_BUILD=1;     shift   ;;
    -h|--help)           usage; exit 0 ;;
    *) echo "Unknown option: $1" >&2; usage >&2; exit 1 ;;
  esac
done

# ── validation ─────────────────────────────────────────────────────────────────
[[ -z "$HOST" ]]         && { echo "error: --host is required" >&2; exit 1; }
[[ -z "$PASSWORD" ]]     && { echo "error: --password is required" >&2; exit 1; }
[[ -z "$BFS_PASSWORD" ]] && { echo "error: --bfs-password is required" >&2; exit 1; }

SCOPE_UPPER="${SCOPE^^}"
if [[ "$SCOPE_UPPER" != "SESSION" && "$SCOPE_UPPER" != "SYSTEM" ]]; then
  echo "error: --scope must be SESSION or SYSTEM" >&2; exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# ── step 1: build ──────────────────────────────────────────────────────────────
if [[ "$SKIP_BUILD" -eq 0 ]]; then
  echo "==> Building slc-rs-slim:dev …"
  # The Dockerfile references an exarrow-rs build context. When crates.io
  # supplies the crate (no local path patch), an empty temp dir satisfies
  # Docker's COPY --from= instruction without patching Cargo.toml.
  _TMP_BFS_CTX=$(mktemp -d)
  trap 'rm -rf "$_TMP_BFS_CTX"' EXIT
  docker build \
    --build-context "exarrow-rs=$_TMP_BFS_CTX" \
    -t slc-rs-slim:dev \
    "$REPO_ROOT"
  echo "==> Build complete."
else
  echo "==> Skipping build (--skip-build); using existing slc-rs-slim:dev."
fi

# ── step 2: export container filesystem ───────────────────────────────────────
echo "==> Exporting container filesystem …"
TMP_TAR=$(mktemp /tmp/slc-XXXXXX.tar.gz)
trap 'rm -f "$TMP_TAR"' EXIT
CID=$(docker create slc-rs-slim:dev)
docker export "$CID" | gzip > "$TMP_TAR"
docker rm "$CID" > /dev/null
echo "==> Exported to $TMP_TAR ($(du -sh "$TMP_TAR" | cut -f1))."

# ── step 3: upload to BucketFS ────────────────────────────────────────────────
BFS_PATH="slc/${SLC_NAME}.tar.gz"
echo "==> Uploading to BucketFS: ${BFS_SERVICE}/${BUCKET}/${BFS_PATH} …"
exapump bucketfs cp "$TMP_TAR" "$BFS_PATH" \
  --bfs-host "$HOST" \
  --bfs-port "$BFS_PORT" \
  --bfs-bucket "$BUCKET" \
  --bfs-write-password "$BFS_PASSWORD" \
  --bfs-tls true \
  --bfs-validate-certificate false
echo "==> Upload complete."

# ── step 4: register SCRIPT_LANGUAGES ─────────────────────────────────────────
SCRIPT_LANGUAGES="RUST=localzmq+protobuf:///${BFS_SERVICE}/${BUCKET}/slc/${SLC_NAME}?lang=rust#buckets/${BFS_SERVICE}/${BUCKET}/slc/${SLC_NAME}/exaudf/exaudfclient"
DSN="exasol://${USER}:${PASSWORD}@${HOST}:${PORT}?validateservercertificate=0"

echo "==> Registering RUST language (ALTER ${SCOPE_UPPER} SET SCRIPT_LANGUAGES) …"
exapump sql \
  "ALTER ${SCOPE_UPPER} SET SCRIPT_LANGUAGES='${SCRIPT_LANGUAGES}'" \
  -d "$DSN"
echo "==> Done. The RUST script language is now available."
echo
echo "    SCRIPT_LANGUAGES entry:"
echo "    ${SCRIPT_LANGUAGES}"
