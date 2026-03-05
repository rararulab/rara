# =============================================================================
# Backend Dockerfile — cargo-chef for dependency caching
# Base image: ghcr.io/crrow/rara-base (built from docker/base.Dockerfile)
# =============================================================================
# syntax=docker/dockerfile:1

ARG BASE_IMAGE=ghcr.io/crrow/rara-base:latest
FROM ${BASE_IMAGE} AS chef

# ---------- Step 1: generate recipe (only depends on Cargo.toml/Cargo.lock) --
FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# ---------- Step 2: build dependencies (cached if recipe unchanged) ----------
FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target \
    cargo chef cook --release --recipe-path recipe.json -p rara-cli

# ---------- Step 3: build source (only recompiles project crates) ------------
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target \
    cargo build -p rara-cli --release && \
    cp target/release/rara /tmp/rara

# ---------- Runtime ----------------------------------------------------------
FROM debian:bookworm-slim AS runtime
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*
RUN useradd -r -m -s /usr/sbin/nologin appuser

COPY --from=builder /tmp/rara /usr/local/bin/rara
COPY --from=builder /app/crates/rara-model/migrations /app/migrations
COPY docker/config.container.yaml /app/config.yaml
RUN chown appuser:appuser /usr/local/bin/rara /app/config.yaml

USER appuser
WORKDIR /app
EXPOSE 25555 50051

CMD ["rara", "server"]
