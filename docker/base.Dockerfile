# =============================================================================
# Base image — Rust toolchain + system deps + cargo-chef (rebuild rarely)
# =============================================================================
# syntax=docker/dockerfile:1

ARG RUST_TOOLCHAIN=stable

FROM rust:bookworm

ARG RUST_TOOLCHAIN

# Pin toolchain if specified
RUN rustup default ${RUST_TOOLCHAIN}

# System dependencies for building crates
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libdbus-1-dev \
    protobuf-compiler \
    libprotobuf-dev \
    && rm -rf /var/lib/apt/lists/*

# cargo-chef for dependency caching
RUN cargo install cargo-chef

WORKDIR /app
