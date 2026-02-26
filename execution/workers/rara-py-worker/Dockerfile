ARG BASE_IMAGE=debian:bookworm-slim
FROM ${BASE_IMAGE}

ARG PYTHON_VERSION=3.11

WORKDIR /app

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*

RUN curl -LsSf https://astral.sh/uv/install.sh | sh \
    && UV_BIN="$(find /root -type f -path '*/bin/uv' | head -n 1)" \
    && test -n "$UV_BIN" \
    && ln -sf "$UV_BIN" /usr/local/bin/uv \
    && if [ -x "$(dirname "$UV_BIN")/uvx" ]; then ln -sf "$(dirname "$UV_BIN")/uvx" /usr/local/bin/uvx; fi \
    && uv --version

ENV UV_PYTHON_INSTALL_DIR=/app/.uv-python
ENV UV_PYTHON_PREFERENCE=only-managed

COPY execution/workers/rara-py-worker /app
COPY api-generated/python/pb /app/api-generated/python/pb

RUN uv python install ${PYTHON_VERSION}
RUN uv sync --frozen --no-dev || uv sync --no-dev

ENV PYTHONPATH=/app/src:/app/api-generated/python/pb
EXPOSE 8080 50051

CMD ["uv", "run", "python", "-m", "python_worker.app.server"]
