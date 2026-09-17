# Feature: slim-image-contents

Asserts what the packaged SLC tarball actually contains: the curated runtime surface, the `/conf` resolver symlinks, the IANA zoneinfo database, and the sandbox mount-point skeleton. This is a packaging/structural contract, distinct from the author-facing library-surface and glibc-floor contract specified in `container/slc-platform-contract`. The build mechanics that produce the staged tree these assertions inspect are specified in `container/slim-image`, not here.

## Background

The Exasol engine sets `TZ` from the session timezone for every UDF (via `NSEXEC_ENV_TZ` → `TZ`), commonly as an IANA name such as `Europe/Berlin`. The staged tree must carry the IANA zoneinfo database so `chrono::Local`/`time` resolve named zones instead of silently falling back to UTC; the runtime never reads `TZ` itself.

The SLC is distributed as a flattened root-filesystem tarball that Exasol extracts after BucketFS upload, with the executable at `/exaudf/exaudfclient`. For DNS to work inside the UDF sandbox, the tarball must present `/etc/hosts` and `/etc/resolv.conf` as symlinks into `/conf/`, which the database populates at runtime. These symlinks cannot be baked as live symlinks in the image layers (`COPY` dereferences a dangling symlink into a 0-byte file; `RUN ln -sf` hits Docker's build-time bind-mount of those two paths), so they are created in a staging directory and tarred inside the Docker build itself.

The `cp -L` staging loop creates only the directories the curated library surface needs, so the staged tree is not a complete root filesystem. Some hosts present the extracted tree to the UDF sandbox as a read-only mount with no writable layer over it. On such a host the sandbox setup cannot create a mount point it does not find, and the reported error is `cannot create directories: Read-only file system` on the first absent path, followed by a bare `22002 VM crashed` for every UDF call. A full-distribution SLC never reaches that error because it already carries the whole skeleton. The staged tree therefore ships the same mount-point skeleton as empty directories. The directory names are committed once, in a single file that both the container build and the tarball contract test read, because a second hand-maintained copy is how the shipped skeleton and the checked skeleton drift apart.

## Scenarios

### Scenario: Staged tarball carries only the curated runtime surface

* *GIVEN* the SLC tarball
* *WHEN* its entries are enumerated
* *THEN* it MUST NOT contain a shell, a package manager or a coreutils binary — no `bin/sh`, no `usr/bin/apt`, no `usr/bin/dpkg`
* *AND* the compressed tarball MUST stay under a committed size ceiling, so accidentally staging the donor's whole root filesystem or its full library surface fails the pipeline instead of shipping
* *AND* the check that enforces the ceiling MUST also report the measured compressed and uncompressed sizes, so any deliberate growth is a visible, reviewed number rather than a silent drift

### Scenario: SLC tarball ships the /conf resolver symlinks

* *GIVEN* the SLC distribution tarball produced from the root `Dockerfile` by the Docker build alone, without any host-side post-processing step
* *WHEN* the entries for `etc/hosts` and `etc/resolv.conf` are inspected
* *THEN* `etc/hosts` MUST be a symbolic-link entry pointing to `/conf/hosts`
* *AND* `etc/resolv.conf` MUST be a symbolic-link entry pointing to `/conf/resolv.conf`
* *AND* producing the tarball MUST NOT require any interpreter or tool outside the Docker build environment (no host `python3`)
* *AND* the tarball MUST be produced with GNU `tar --hard-dereference`, so no shipped path depends on BucketFS extraction recreating a hard link

### Scenario: Runtime image bundles the IANA zoneinfo database

* *GIVEN* the SLC tarball and that the database always sends the session timezone as `TZ` for every UDF
* *WHEN* the tarball is inspected for the zoneinfo database
* *THEN* `usr/share/zoneinfo/Europe/Berlin` MUST be present as a readable, non-empty regular file, not a link whose target could be lost in extraction
* *AND* the fix MUST remain packaging only (an `apt-get install` of `tzdata`), since `chrono`/`time` consult the zoneinfo database implicitly and the runtime MUST NOT read `TZ` itself

### Scenario: SLC tarball ships the sandbox mount-point skeleton

* *GIVEN* the SLC distribution tarball
* *WHEN* its directory entries are enumerated
* *THEN* the tarball MUST contain `boot`, `buckets`, `conf`, `dev`, `home`, `media`, `mnt`, `opt`, `proc`, `root`, `run`, `scripts`, `srv`, `sys`, `var/cache`, `var/lib`, `var/local`, `var/log`, `var/opt`, `var/spool` and `var/tmp` as directory entries, so a host that mounts the extracted tree read-only finds every mount point it needs already present
* *AND* the skeleton MUST add directories only, each empty except where the list names a subdirectory under it, and MUST add no regular file, symbolic link, device node or socket, so the curated-runtime-surface and size-ceiling assertions stay satisfied
* *AND* the container build and the tarball contract test MUST both read those names from one committed file rather than each carry a list of its own, so a name added to that file alone is both shipped and asserted
* *AND* a name in that committed file that the tarball does not carry MUST fail the contract test
