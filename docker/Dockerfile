# =============================================================================
# Backend Dockerfile — multi-stage build with cargo-chef for dependency caching
# =============================================================================

# ---------------------------------------------------------------------------
# Stage 1: chef — install cargo-chef on the Rust base image
# ---------------------------------------------------------------------------
FROM rust:1.86-bookworm AS chef
RUN cargo install cargo-chef
WORKDIR /app

# ---------------------------------------------------------------------------
# Stage 2: planner — generate the dependency recipe
# ---------------------------------------------------------------------------
FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# ---------------------------------------------------------------------------
# Stage 3: builder — cook dependencies then build the application
# ---------------------------------------------------------------------------
FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json
COPY . .
RUN cargo build -p rara-cli --release

# ---------------------------------------------------------------------------
# Stage 4: runtime — minimal Debian image with only runtime dependencies
# ---------------------------------------------------------------------------
FROM debian:bookworm-slim AS runtime
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*
RUN useradd -r -m -s /usr/sbin/nologin appuser

COPY --from=builder /app/target/release/job /usr/local/bin/job
RUN chown appuser:appuser /usr/local/bin/job

USER appuser
WORKDIR /app
EXPOSE 25555 50051

CMD ["job", "server"]
