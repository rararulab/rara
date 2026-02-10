ARG RUST_TOOLCHAIN=stable
ARG TARGET_PLATFORM=linux/amd64

FROM ubuntu:25.04 as base
ENV LANG en_US.utf8
WORKDIR /app

RUN apt-get update && \
    DEBIAN_FRONTEND=noninteractive apt-get install -y software-properties-common
# Install dependencies.
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y \
    libssl-dev \
    tzdata \
    curl \
    unzip \
    ca-certificates \
    git \
    build-essential \
    pkg-config 

RUN echo "target platform: $TARGET_PLATFORM"
RUN echo "rust toolchain: $RUST_TOOLCHAIN"

# Install Rust.
SHELL ["/bin/bash", "-c"]
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- --no-modify-path --default-toolchain none -y
ENV PATH /root/.cargo/bin/:$PATH

# Install Rust toolchains.
ARG RUST_TOOLCHAIN=stable
RUN rustup toolchain install ${RUST_TOOLCHAIN}

# Install binstall
RUN curl -L --proto '=https' --tlsv1.2 -sSf https://raw.githubusercontent.com/cargo-bins/cargo-binstall/main/install-from-binstall-release.sh | bash

# Install nextest, just
RUN cargo binstall cargo-nextest just --no-confirm

# Install sccache
RUN cargo install sccache cargo-chef

FROM base as planner

ENV RUSTC_WRAPPER=sccache SCCACHE_DIR=/sccache
WORKDIR /app
COPY . .

RUN --mount=type=cache,target=$SCCACHE_DIR,sharing=locked \
    cargo chef prepare --recipe-path recipe.json

FROM base as builder

ENV RUSTC_WRAPPER=sccache SCCACHE_DIR=/sccache
WORKDIR /app
COPY --from=planner /app/recipe.json recipe.json

RUN --mount=type=cache,target=$SCCACHE_DIR,sharing=locked \
    cargo chef cook --release --recipe-path recipe.json
    
COPY . .
RUN --mount=type=cache,target=$SCCACHE_DIR,sharing=locked \
    just build

# Use a minimal runtime image for the final stage
FROM debian:bookworm-slim as runtime

# Install minimal runtime dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

# Copy the built binary
COPY --from=builder /app/target/release/job /usr/local/bin/job

# Create a non-root user for security
RUN useradd -r -s /usr/sbin/nologin appuser
USER appuser

WORKDIR /app

CMD ["job", "server"]
