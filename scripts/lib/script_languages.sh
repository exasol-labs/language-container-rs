#!/usr/bin/env bash
# Shared assembly of the Exasol SCRIPT_LANGUAGES entry for the RUST alias.
# Sourced by scripts/install.sh (both its HTTP-BucketFS and Personal-over-SSH
# transports); it defines functions and constants only, so sourcing it more
# than once is harmless. It assembles a single entry and nothing else — how that
# entry reaches the database (overwrite via ALTER SESSION, or append via ALTER
# SYSTEM) is each install path's own decision.

SCRIPT_LANGUAGE_ALIAS=RUST

# Assemble the registration entry for an SLC extracted at
# <bfs-service>/<bucket>/<slc-path> in BucketFS.
#
# The `#` fragment MUST name the exaudfclient EXECUTABLE — not its directory —
# and MUST carry no leading slash. Either mistake leaves the engine unable to
# start the UDF client, and every UDF dies as a bare `22002 VM crashed` with no
# further diagnostic.
#
# crates/it/src/lib.rs SlcRef::script_languages independently rebuilds this
# same registration-string format for the integration-test harness; keep the
# two in sync if the format or the executable-path/no-leading-slash invariant
# ever changes.
script_languages_entry() {
  local bfs_service="$1" bucket="$2" slc_path="$3"

  if [[ -z "$bfs_service" || -z "$bucket" || -z "$slc_path" ]]; then
    echo "error: script_languages_entry needs a BucketFS service, a bucket and an SLC path" >&2
    return 1
  fi

  printf '%s=localzmq+protobuf:///%s/%s/%s?lang=rust#buckets/%s/%s/%s/exaudf/exaudfclient\n' \
    "$SCRIPT_LANGUAGE_ALIAS" \
    "$bfs_service" "$bucket" "$slc_path" \
    "$bfs_service" "$bucket" "$slc_path"
}
