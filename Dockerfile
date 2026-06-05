# Stage 1: Builder
FROM rust:1.84-bookworm AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
    libzmq3-dev \
    protobuf-compiler \
    pkg-config \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build

# Copy workspace files
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates/ ./crates/
COPY test-udfs/ ./test-udfs/
COPY targets/ ./targets/

# Copy exarrow-rs (path dependency patched in Cargo.toml)
# Provided via: --build-context exarrow-rs=/home/talos/code/exarrow-rs
COPY --from=exarrow-rs . /exarrow-rs/

# Rewrite the [patch.crates-io] path to the container-local location
RUN sed -i 's|path = "/home/talos/code/exarrow-rs"|path = "/exarrow-rs"|g' Cargo.toml

# Build the exaudfclient binary
RUN cargo build --release -p exaudfclient

# Stage 2: Runtime
FROM debian:12-slim AS runtime

RUN apt-get update && apt-get install -y --no-install-recommends \
    libzmq5 \
    ca-certificates \
    locales \
    && locale-gen en_US.UTF-8 \
    && rm -rf /var/lib/apt/lists/*

ENV LANG=en_US.UTF-8 \
    LC_ALL=en_US.UTF-8

RUN mkdir -p /exaudf

COPY --from=builder /build/target/release/exaudfclient /exaudf/exaudfclient
RUN chmod +x /exaudf/exaudfclient

# Copy language definitions
COPY build_info/ /build_info/

ENTRYPOINT ["/exaudf/exaudfclient"]
