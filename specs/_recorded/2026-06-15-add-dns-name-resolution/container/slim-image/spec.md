# Feature: slim-image

Packages the `exaudfclient` binary into a slim Alpine-based SLC Docker image (Option A only, no Rust toolchain) that Exasol can register as a `localzmq+protobuf` language container.

## Background

The SLC is distributed as a flattened root-filesystem tarball that Exasol extracts after BucketFS upload, with the executable at `/exaudf/exaudfclient`. For DNS to work inside the UDF sandbox, the tarball must present `/etc/hosts` and `/etc/resolv.conf` as symlinks into `/conf/`, which the database populates at runtime. These symlinks cannot be baked as live symlinks in the image layers (`COPY` dereferences a dangling symlink into a 0-byte file; `RUN ln -sf` hits Docker's build-time bind-mount of those two paths), so they are created in a staging directory and tarred inside the Docker build itself.

## Scenarios

<!-- DELTA:NEW -->
### Scenario: SLC tarball ships the /conf resolver symlinks

* *GIVEN* the SLC distribution tarball produced from `Dockerfile.alpine` by the Docker build alone, without any host-side post-processing step
* *WHEN* the entries for `etc/hosts` and `etc/resolv.conf` are inspected
* *THEN* `etc/hosts` MUST be a symbolic-link entry pointing to `/conf/hosts`
* *AND* `etc/resolv.conf` MUST be a symbolic-link entry pointing to `/conf/resolv.conf`
* *AND* producing the tarball MUST NOT require any interpreter or tool outside the Docker build environment (no host `python3`)
<!-- /DELTA:NEW -->
