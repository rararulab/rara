# syntax=docker/dockerfile:1
#
# Server-only, sandbox-disabled container image for the rara backend.
#
# This image ships ONLY the `rara server` control-plane process — no web
# bundle, no nginx, no boxlite/microVM sandbox runtime. Untrusted-code
# execution is deliberately NOT solved here; it is deferred to the fleet
# microVM layer (see issue #2239 and the fleet direction, 2026-07-05).
#
# Target architecture: linux/amd64 ONLY. Both stages pin
# `--platform=linux/amd64` so a bare `docker build` (no --platform flag)
# still produces an amd64 image. arm64 is out of scope (doubles CI cost
# and needs gcc-13 for the numkong SIMD build).
#
# Build:
#   docker build -f docker/Dockerfile -t rara-server:smoke .
#
# The build is a full-workspace `cargo build --release` — expect it to be
# slow on the first run. GitHub-hosted x64 runners build it natively; on an
# arm64 dev machine it runs under QEMU emulation (slower still).

# ---------------------------------------------------------------------------
# Builder — ubuntu:24.04 (gcc-13). Do NOT downgrade to bookworm/gcc-12:
# numkong's SIMD code fails to compile there.
# ---------------------------------------------------------------------------
FROM --platform=linux/amd64 ubuntu:24.04 AS builder

ENV DEBIAN_FRONTEND=noninteractive

# System toolchain. This mirrors `.github/actions/setup-rara-build` plus the
# extra bits a from-scratch container needs (git, curl, xz, ca-certificates):
#   - libsqlite3-dev / libpq-dev : diesel (libsqlite3-sys, pq-sys) headers
#   - protobuf-compiler          : api/build.rs (prost-build)
#   - clang + mold               : `.cargo/config.toml` forces
#                                  `-C link-arg=-fuse-ld=mold` + `linker=clang`
#                                  on x86_64-unknown-linux-gnu
#   - libssl-dev                 : openssl-sys (pulled in transitively via
#                                  pq-sys/libpq); make the dep explicit
#   - zlib1g-dev + pkg-config    : zlob / native deps
RUN apt-get update && apt-get install -y --no-install-recommends \
        build-essential \
        clang \
        mold \
        protobuf-compiler \
        libsqlite3-dev \
        libpq-dev \
        libssl-dev \
        zlib1g-dev \
        pkg-config \
        git \
        curl \
        xz-utils \
        ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Rust toolchain via rustup. `rust-toolchain.toml` (channel = stable) is
# honored automatically once the source tree is copied in, so we only need a
# working rustup + stable here.
ENV RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/cargo/bin:$PATH
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
      | sh -s -- -y --default-toolchain stable --profile minimal

# Zig 0.16.0 — required unconditionally by `crates/app`'s zlob build.rs.
# Not available via apt; download the official release tarball and put it on
# PATH. Version kept in lockstep with .github/workflows/zig-codec-smoke.yml.
ARG ZIG_VERSION=0.16.0
RUN curl -fsSL "https://ziglang.org/download/${ZIG_VERSION}/zig-x86_64-linux-${ZIG_VERSION}.tar.xz" \
      | tar -xJ -C /opt \
    && ln -s "/opt/zig-x86_64-linux-${ZIG_VERSION}/zig" /usr/local/bin/zig \
    && zig version

WORKDIR /build
COPY . .

# BOXLITE_DEPS_STUB=1 stubs out the native boxlite dependency so the sandbox
# subsystem builds without its microVM runtime. The sandbox is disabled in
# this image (no sandbox block in the shipped config); its tools stay
# registered and return a clear "sandbox not configured" error at runtime.
ENV BOXLITE_DEPS_STUB=1

# Build the CLI (`rara` binary) in release mode and stage it. The cache
# mounts speed up local iterative rebuilds; the binary is copied OUT of the
# ephemeral target/ cache within the same RUN so it survives into the layer.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/build/target \
    cargo build --release --locked -p rara-cli \
    && cp target/release/rara /usr/local/bin/rara

# ---------------------------------------------------------------------------
# Runtime — ubuntu:24.04 slim (NOT distroless): rara links libsqlite3, libpq
# and zlib at runtime, so their shared objects must be present.
# ---------------------------------------------------------------------------
FROM --platform=linux/amd64 ubuntu:24.04 AS runtime

ENV DEBIAN_FRONTEND=noninteractive

# Runtime shared objects + curl (for HEALTHCHECK). Dedicated non-root user.
RUN apt-get update && apt-get install -y --no-install-recommends \
        libsqlite3-0 \
        libpq5 \
        libssl3 \
        zlib1g \
        ca-certificates \
        curl \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 10001 rara \
    && useradd --uid 10001 --gid 10001 --home-dir /home/rara \
        --create-home --shell /usr/sbin/nologin rara \
    && mkdir -p /config/rara /data /run/rara \
    && chown -R rara:rara /home/rara /config /data /run/rara

COPY --from=builder /usr/local/bin/rara /usr/local/bin/rara

# Path wiring (rara's `rara-paths` crate honors XDG on Linux; `dirs` reads
# XDG_CONFIG_HOME / XDG_DATA_HOME / HOME):
#   - config (base, non-secret) : $XDG_CONFIG_HOME/rara/config.yaml
#                                 -> /config/rara/config.yaml  (ConfigMap)
#   - config (secret overlay)   : $CWD/config.yaml
#                                 -> /run/rara/config.yaml     (Secret)
#   - data (SQLite DB, sessions): $XDG_DATA_HOME/rara/...
#                                 -> /data/rara/...            (PVC)
# rara natively merges the two config files (global then local; local wins),
# so secrets never need to live in the image or the ConfigMap.
ENV HOME=/home/rara \
    XDG_CONFIG_HOME=/config \
    XDG_DATA_HOME=/data

USER rara
# CWD holds the optional secret overlay (./config.yaml). Empty when no
# Secret is mounted, in which case only the base config is used.
WORKDIR /run/rara

# 25555 = HTTP (REST + WS), 50051 = gRPC. 25556 (gateway admin) is NOT
# exposed — it stays pod-internal loopback.
EXPOSE 25555 50051

HEALTHCHECK --interval=15s --timeout=5s --start-period=45s --retries=5 \
    CMD curl -sf --max-time 4 http://127.0.0.1:25555/api/health | grep -q healthy || exit 1

# Run the server directly. In a pod the kubelet is the supervisor, so we do
# NOT wrap it in the `rara gateway` process supervisor.
ENTRYPOINT ["rara"]
CMD ["server"]
