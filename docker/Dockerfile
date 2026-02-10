ARG BASE_IMAGE=job-base:latest

FROM ${BASE_IMAGE} AS planner
ENV RUSTC_WRAPPER=sccache SCCACHE_DIR=/sccache
WORKDIR /app
COPY . .
RUN --mount=type=cache,target=$SCCACHE_DIR,sharing=locked \
    cargo chef prepare --recipe-path recipe.json

FROM ${BASE_IMAGE} AS builder
ENV RUSTC_WRAPPER=sccache SCCACHE_DIR=/sccache
WORKDIR /app
COPY --from=planner /app/recipe.json recipe.json
RUN --mount=type=cache,target=$SCCACHE_DIR,sharing=locked \
    cargo chef cook --release --recipe-path recipe.json
COPY . .
RUN --mount=type=cache,target=$SCCACHE_DIR,sharing=locked \
    cargo build -p job-cli --release

# Minimal runtime image (needs Python for PyO3/jobspy-sys)
FROM ubuntu:25.04 AS runtime
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    libssl3t64 \
    libpython3.13 \
    && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/job /usr/local/bin/job
COPY --from=builder /root/.cache/jobspy /root/.cache/jobspy
RUN useradd -r -s /usr/sbin/nologin appuser
USER appuser
WORKDIR /app
CMD ["job", "server"]
