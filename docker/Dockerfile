# =============================================================================
# Backend Dockerfile — cargo-chef for dependency caching
# Requires: just build-base (builds rara-base:latest with toolchain + deps)
# =============================================================================
# syntax=docker/dockerfile:1

FROM rara-base:latest AS chef

# ---------- Step 1: generate recipe (only depends on Cargo.toml/Cargo.lock) --
FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# ---------- Step 2: build dependencies (cached if recipe unchanged) ----------
FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json -p rara-cli

# ---------- Step 3: build source (only recompiles project crates) ------------
COPY . .
RUN cargo build -p rara-cli --release && \
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
RUN chown appuser:appuser /usr/local/bin/rara

USER appuser
WORKDIR /app
EXPOSE 25555 50051

CMD ["rara", "server"]
