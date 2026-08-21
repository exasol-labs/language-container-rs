# Stage 1: Builder — compiles the client on the same Debian release the staging
# stage donates its runtime from, so the shipped binary and the bundled glibc match.
FROM rust:1.94-trixie AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
    protobuf-compiler \
    pkg-config \
    && rm -rf /var/lib/apt/lists/*

# No libzmq3-dev: zmq-sys falls back to zeromq-src (static zmq), eliminating
# the libzmq runtime dependency from the exported binary.
# Force bzip2-sys to build from vendored source instead of linking the system
# libbz2: rust:1.94-trixie ships libbz2-dev on aarch64 but not x86_64, so
# without this pin the aarch64 binary picks up a DT_NEEDED for libbz2.so.1.0.
ENV BZIP2_NO_PKG_CONFIG=1

WORKDIR /build

COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates/ ./crates/
COPY test-udfs/ ./test-udfs/
COPY benches/ ./benches/

# The workspace toolchain pin and the image toolchain are both 1.94; drop the
# pin so the image's own 1.94 toolchain is used (no version split).
RUN rm rust-toolchain.toml

RUN cargo build --release -p exaudfclient

# The staging stage has neither binutils nor dpkg-architecture, so the two
# architecture-dependent values are derived here and handed over as files:
# TRIPLET is the Debian multiarch directory name (x86_64-linux-gnu on amd64,
# aarch64-linux-gnu on arm64) and LOADER is the built binary's own PT_INTERP
# (/lib64/ld-linux-x86-64.so.2 on amd64, /lib/ld-linux-aarch64.so.1 on arm64).
# The committed library surface is handed over the same way, so the staging loop
# below stages exactly the sonames `cargo exasol-udf validate` accepts.
RUN TRIPLET="$(gcc -print-multiarch)" && \
    LOADER="$(readelf -l /build/target/release/exaudfclient \
        | sed -n 's/.*interpreter: \(.*\)]/\1/p' | tr -d ' ')" && \
    if [ -z "$TRIPLET" ]; then \
        echo "error: empty multiarch triplet from 'gcc -print-multiarch'" >&2; \
        exit 1; \
    fi && \
    if [ -z "$LOADER" ]; then \
        echo "error: empty PT_INTERP loader path from 'readelf -l /build/target/release/exaudfclient'" >&2; \
        exit 1; \
    fi && \
    mkdir -p /slc-meta && \
    printf '%s\n' "$TRIPLET" > /slc-meta/triplet && \
    printf '%s\n' "$LOADER" > /slc-meta/loader && \
    cp /build/crates/cargo-exasol-udf/slc-library-surface.txt /slc-meta/library-surface

# Stage 2: Staging — the runtime donor and the packager in one. The extracted
# tree is the UDF's entire root filesystem, so only the documented library
# surface is staged: no shell, no package manager, no coreutils.
FROM debian:trixie-slim AS staging

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    tzdata \
    && rm -rf /var/lib/apt/lists/*

# Every other staged library already ships in this base image; naming a package
# for one would also hit the time64 rename (libssl3 -> libssl3t64).

ENV LANG=C.UTF-8

COPY --from=builder /slc-meta/ /slc-meta/

RUN TRIPLET="$(cat /slc-meta/triplet)" && \
    LOADER="$(cat /slc-meta/loader)" && \
    mkdir -p /slc && \
    # Reproduce the donor's own usr-merge links, and the real directory behind
    # each, before any file is staged: cp -L then writes through the links so
    # every real file lands under /slc/usr instead of a directory that would
    # shadow a link of the same name.
    for d in lib lib64 bin sbin; do \
        if [ -L "/$d" ]; then \
            target="$(readlink "/$d")" && \
            ln -s "$target" "/slc/$d" && \
            mkdir -p "/slc/$target"; \
        fi; \
    done && \
    LIBDIR="/slc/usr/lib/$TRIPLET" && \
    mkdir -p "$LIBDIR/ossl-modules" "$LIBDIR/engines-3" \
             "/slc$(dirname "$LOADER")" \
             /slc/usr/lib/locale /slc/usr/lib/ssl /slc/usr/share \
             /slc/etc/ld.so.conf.d /slc/etc/ssl/certs /slc/tmp && \
    # The client sets HOME=/tmp and writes its startup and connect-back traces
    # there; those writes are best-effort, so a missing /tmp would only show up
    # as silently absent diagnostics inside the sandbox.
    chmod 1777 /slc/tmp && \
    cp -L "$LOADER" "/slc$LOADER" && \
    # Staged anywhere but its own PT_INTERP path the loader dangles after
    # BucketFS extraction and every UDF dies as a bare 22002 VM crashed.
    if [ ! -f "/slc$LOADER" ]; then \
        echo "error: loader not staged at PT_INTERP path /slc$LOADER" >&2; \
        exit 1; \
    fi && \
    for lib in $(cat /slc-meta/library-surface); do \
        cp -L "/usr/lib/$TRIPLET/$lib" "$LIBDIR/$lib" || exit 1; \
    done && \
    cp -L "/usr/lib/$TRIPLET/ossl-modules/"*.so "$LIBDIR/ossl-modules/" && \
    cp -L "/usr/lib/$TRIPLET/engines-3/"*.so "$LIBDIR/engines-3/" && \
    cp -RL /usr/lib/locale/C.utf8 /slc/usr/lib/locale/ && \
    cp -a /usr/share/zoneinfo /slc/usr/share/ && \
    cp -L /etc/ssl/certs/ca-certificates.crt /slc/etc/ssl/certs/ && \
    # OpenSSL's built-in default trust path is /usr/lib/ssl/{cert.pem,certs},
    # both of which the donor ships as links into /etc/ssl/certs.
    cp -a /usr/lib/ssl/cert.pem /usr/lib/ssl/certs /slc/usr/lib/ssl/ && \
    printf 'passwd: files\ngroup: files\nshadow: files\nhosts: files dns\nnetworks: files\nprotocols: files\nservices: files\n' \
        > /slc/etc/nsswitch.conf && \
    printf '/lib/%s\n/usr/lib/%s\n' "$TRIPLET" "$TRIPLET" \
        > "/slc/etc/ld.so.conf.d/$TRIPLET.conf" && \
    printf 'include /etc/ld.so.conf.d/*.conf\n' > /slc/etc/ld.so.conf && \
    # Also writes the soname link for any staged file whose SONAME differs from
    # its file name (libbz2.so.1's soname is libbz2.so.1.0), which is what the
    # loader looks up, so the mapping is never hardcoded here.
    ldconfig -r /slc

RUN mkdir -p /slc/exaudf /slc/build_info

COPY --from=builder /build/target/release/exaudfclient /slc/exaudf/exaudfclient
RUN chmod +x /slc/exaudf/exaudfclient

COPY build_info/ /slc/build_info/

# License + third-party attribution shipped with the distributed binary
# (permissive deps require reproducing their notices in binary redistribution).
# Both THIRD-PARTY-*.md files are generated by dist/generate-licenses.sh and are
# git-ignored, so they must exist in dist/ in the build context at build time
# (the script is run before this build — see scripts/install.sh and CI):
#   THIRD-PARTY-LICENSES.md     — Rust crate graph (cargo about + about.hbs)
#   THIRD-PARTY-OS-LICENSES.md  — Staged libraries (cargo about + os-licenses.hbs)
COPY LICENSE /slc/exaudf/
COPY dist/THIRD-PARTY-LICENSES.md dist/THIRD-PARTY-OS-LICENSES.md /slc/exaudf/

RUN ln -sf /conf/hosts /slc/etc/hosts && \
    ln -sf /conf/resolv.conf /slc/etc/resolv.conf

# The staged tree is the UDF's entire root filesystem, so this proves the
# client is loadable inside it before the tarball ever ships: a no-argument
# invocation returns instantly with exit 1 and a wrong-argument-count message,
# unlike an endpoint argument, which polls for 120s before failing.
RUN set +e; \
    OUTPUT="$(chroot /slc /exaudf/exaudfclient 2>&1)"; \
    STATUS=$?; \
    set -e; \
    if [ "$STATUS" -eq 0 ]; then \
        echo "error: chroot self-test exited 0, expected non-zero" >&2; \
        exit 1; \
    fi; \
    case "$OUTPUT" in \
        *"wrong argument count"*) ;; \
        *) \
            echo "error: chroot self-test output missing wrong-argument-count message:" >&2; \
            echo "$OUTPUT" >&2; \
            exit 1 ;; \
    esac

# The self-test's own run wrote its traces into /slc/tmp; remove every entry,
# whatever it is named, so the shipped tarball's /tmp carries no build-time
# content while keeping the directory itself and its 1777 mode.
RUN find /slc/tmp -mindepth 1 -delete

RUN tar --hard-dereference -C /slc -czf /lc-rs.tar.gz .

# Stage 3: Artifact — expose the SLC tarball for docker build --output
FROM scratch AS artifact

COPY --from=staging /lc-rs.tar.gz /
