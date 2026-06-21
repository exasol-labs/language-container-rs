#!/usr/bin/env bash
#
# ci-it-local.sh — replay the GitHub `integration` job locally.
#
# Mirrors .github/workflows/ci.yml's integration matrix leg end-to-end so the
# "VM crashed" failure can be reproduced and the fix validated WITHOUT spending
# a CI run. The Docker `--memory` cgroup cap bites regardless of host RAM, so
# the sandbox-starvation bug reproduces even on a big dev box.
#
# Usage:
#   # reproduce the bug (broken CI config: 6g cap, DB RAM auto-sized):
#   scripts/ci-it-local.sh
#
#   # validate the fix (pin DB RAM, generous ceiling):
#   DB_MEM='4 GiB' MEM=12g SHM=2g scripts/ci-it-local.sh
#
# Env knobs (defaults reproduce the *broken* pre-fix config):
#   MEM             docker --memory          (default: 6g)
#   MEMSWAP         docker --memory-swap      (default: =MEM, i.e. swap disabled,
#                                              to mimic a swap-starved CI runner;
#                                              set MEMSWAP=-1 for unlimited swap)
#   SHM             docker --shm-size         (default: 2g)
#   DB_MEM          EXA_DB_MEM_SIZE           (default: unset → docker-db auto-sizes)
#   EXASOL_VERSION  docker-db image tag       (default: 2026.1.0)
#   DB_SERIES       it crate feature          (default: db-2026-1)
#   SKIP_SLC_BUILD  reuse existing SLC tarball (requires SLC_TARBALL)
#   DB_PORT         host port -> DB 8563      (default: 8563)
#   BFS_PORT        host port -> BucketFS 2581 (default: 2581)
#
# NOTE: a dev box with lots of swap will NOT reproduce the CI "VM crashed" with
# the default Docker swap allowance (2x --memory) — the container spills to swap
# and survives. MEMSWAP defaults to =MEM here so the cgroup denies swap, which is
# what actually reproduces the OOM-kill of the UDF sandbox locally.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

MEM="${MEM:-6g}"
MEMSWAP="${MEMSWAP:-$MEM}"
SHM="${SHM:-2g}"
DB_MEM="${DB_MEM:-}"
EXASOL_VERSION="${EXASOL_VERSION:-2026.1.0}"
DB_SERIES="${DB_SERIES:-db-2026-1}"
CONTAINER="exasol-db"
IMAGE="exasol/docker-db:${EXASOL_VERSION}"
# Host port mappings. Override (e.g. DB_PORT=18563 BFS_PORT=12581) to run
# alongside another local Exasol that already holds 8563/2581.
DB_PORT="${DB_PORT:-8563}"
BFS_PORT="${BFS_PORT:-2581}"

log() { printf '\n\033[1;36m=== %s ===\033[0m\n' "$*"; }

cleanup() { docker stop "$CONTAINER" >/dev/null 2>&1 || true; docker rm "$CONTAINER" >/dev/null 2>&1 || true; }
on_exit() { cleanup; rm -rf "${SLC_DIR:-}" 2>/dev/null || true; }
trap on_exit EXIT

log "Config: MEM=$MEM MEMSWAP=$MEMSWAP SHM=$SHM DB_MEM='${DB_MEM:-<auto>}' IMAGE=$IMAGE"

# 1. Build the SLC tarball via the artifact stage ----------------------------
SLC_DIR="${SLC_DIR:-/tmp/lc-rs-$$}"
if [ -z "${SKIP_SLC_BUILD:-}" ]; then
  log "Generate license bundles (dist/generate-licenses.sh)"
  bash "$REPO_ROOT/dist/generate-licenses.sh"
  log "Build SLC tarball (Dockerfile.alpine --target artifact -> $SLC_DIR/lc-rs.tar.gz)"
  mkdir -p "$SLC_DIR"
  docker build -f Dockerfile.alpine --target artifact \
    --output "type=local,dest=$SLC_DIR" .
else
  log "Reusing existing SLC tarball (SKIP_SLC_BUILD set): ${SLC_TARBALL:-<SLC_TARBALL unset>}"
  if [ -z "${SLC_TARBALL:-}" ]; then
    echo "ERROR: SKIP_SLC_BUILD set but SLC_TARBALL is not set"; exit 1
  fi
fi
export SLC_TARBALL="${SLC_TARBALL:-$SLC_DIR/lc-rs.tar.gz}"

# 2. Build the UDF .so artifacts (release) — same set as the build job --------
log "Build UDF .so artifacts (cargo build --release)"
cargo build --release \
  -p scalar-double \
  -p set-filter \
  -p json-parse \
  -p single-call-fixture \
  -p connect-back-cluster-ip \
  -p connect-back-query \
  -p connect-back-scalar \
  -p connect-back-insert \
  -p connect-back-crunch \
  -p resolv-udf \
  -p emit-bulk \
  -p connect-back-stream \
  -p timestamp-add-second \
  -p timestamp-now \
  -p timestamp-passthrough \
  -p annotated-fixture

# 3. Build the IT test binary (it-runner) -------------------------------------
log "Build IT test binary (it-runner)"
BIN=$(cargo test --no-run -p it --features "integration,${DB_SERIES}" \
    --message-format=json 2>/dev/null \
  | jq -r 'select(.reason == "compiler-artifact" and (.target.kind | contains(["test"]))) | .executable' \
  | grep -v '^null$' | head -1)
[ -n "$BIN" ] || { echo "ERROR: could not locate it test binary"; exit 1; }
cp "$BIN" it-runner
chmod +x it-runner

# 4. Start Exasol with the parameterized memory flags -------------------------
log "Start Exasol ($IMAGE)"
cleanup
docker image inspect "$IMAGE" >/dev/null 2>&1 || docker pull "$IMAGE"
DB_MEM_ARG=()
[ -n "$DB_MEM" ] && DB_MEM_ARG=(-e "EXA_DB_MEM_SIZE=$DB_MEM")
docker run -d --name "$CONTAINER" --privileged \
  --shm-size="$SHM" \
  --memory="$MEM" \
  --memory-swap="$MEMSWAP" \
  "${DB_MEM_ARG[@]}" \
  -e COSLWD_ENABLED=1 \
  -p "$DB_PORT:8563" -p "$BFS_PORT:2581" \
  "$IMAGE"

# 5. Wait for SQL (same exapump health probe as CI) ---------------------------
log "Wait for Exasol (exapump health probe)"
ready=
for i in $(seq 1 60); do
  if exapump sql --dsn "exasol://sys:exasol@localhost:$DB_PORT/?validateservercertificate=0" "SELECT 1" >/dev/null 2>&1; then
    echo "Exasol is ready."; ready=1; break
  fi
  echo "Attempt $i/60 failed; retrying in 5s..."; sleep 5
done
[ -n "$ready" ] || { echo "ERROR: Exasol never became ready"; docker logs --tail 50 "$CONTAINER" || true; exit 1; }

# 6. Extract BucketFS write password (same as CI) -----------------------------
log "Extract BucketFS write password"
BFSPASS=$(docker exec "$CONTAINER" bash -c "
  awk '/\[\[Bucket : default\]\]/{f=1} f && /WritePasswd/{print \$3; exit}' /exa/etc/EXAConf \
  | python3 -c 'import sys, base64; print(base64.b64decode(sys.stdin.read().strip()).decode())'
")
[ -n "$BFSPASS" ] || { echo "ERROR: could not extract BucketFS password"; exit 1; }

# 7. Run the integration tests in external mode -------------------------------
log "Run integration tests (./it-runner, external mode)"
set +e
EXASOL_VERSION="$EXASOL_VERSION" \
EXASOL_HOST=localhost EXASOL_PORT=$DB_PORT BUCKETFS_PORT=$BFS_PORT \
BUCKETFS_PASSWORD="$BFSPASS" \
SLC_TARBALL="$SLC_TARBALL" \
./it-runner --nocapture
rc=$?
set -e

if [ "$rc" -ne 0 ]; then
  log "FAILED (rc=$rc) — dumping DB-side abort evidence"
  docker exec "$CONTAINER" sh -c 'find /exa/logs/cored -name "exasql.*" -type f 2>/dev/null | sort | xargs -I{} sh -c "echo === {} ===; tail -40 {}"' || true
  docker exec "$CONTAINER" sh -c 'dmesg 2>/dev/null | grep -iE "oom|killed process" | tail -20' || true
fi

log "Done (rc=$rc)"
exit "$rc"
