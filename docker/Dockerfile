# =============================================================================
# Backend Dockerfile — buildx + BuildKit cache for fast incremental builds
# =============================================================================
# syntax=docker/dockerfile:1

FROM rust:bookworm AS builder
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libdbus-1-dev \
    protobuf-compiler \
    libprotobuf-dev \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/app/target \
    cargo build -p rara-cli --release && \
    cp target/release/rara /tmp/rara

FROM debian:bookworm-slim AS runtime
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*
RUN useradd -r -m -s /usr/sbin/nologin appuser

COPY --from=builder /tmp/rara /usr/local/bin/rara
RUN chown appuser:appuser /usr/local/bin/rara

USER appuser
WORKDIR /app
EXPOSE 25555 50051

CMD ["rara", "server"]
