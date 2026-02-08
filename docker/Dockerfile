ARG RUST_TOOLCHAIN
ARG TARGET_PLATFORM

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
ARG RUST_TOOLCHAIN
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
FROM alpine:3.19 as runtime

# Install minimal runtime dependencies
RUN apk add --no-cache \
    ca-certificates \
    libgcc

# Copy all built binaries and output files from the target folder
COPY --from=builder /app/target/release/ /usr/local/bin/

# Create a non-root user for security
RUN adduser -D -s /bin/sh appuser
USER appuser

WORKDIR /app
