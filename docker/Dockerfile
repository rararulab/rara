ARG BASE_IMAGE=job-base:latest

FROM ${BASE_IMAGE} AS planner
ENV RUSTC_WRAPPER=sccache SCCACHE_DIR=/sccache
WORKDIR /app
COPY . .
RUN --mount=type=cache,target=$SCCACHE_DIR,sharing=locked \
    cargo chef prepare --recipe-path recipe.json

FROM ${BASE_IMAGE} AS builder
ENV RUSTC_WRAPPER=sccache SCCACHE_DIR=/sccache
# Use /home/appuser as HOME so jobspy cache paths match the runtime user
# Keep RUSTUP_HOME and CARGO_HOME at /root where toolchain was installed
ENV HOME=/home/appuser RUSTUP_HOME=/root/.rustup CARGO_HOME=/root/.cargo
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
RUN useradd -r -m -s /usr/sbin/nologin appuser
COPY --from=builder /app/target/release/job /usr/local/bin/job
COPY --from=builder /home/appuser/.cache/jobspy /home/appuser/.cache/jobspy
RUN chown -R appuser:appuser /home/appuser
USER appuser
WORKDIR /app
CMD ["job", "server"]
