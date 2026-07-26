# syntax=docker/dockerfile:1

# Build the Vite application with the Bun version used by CI at the time this
# image was introduced. The manifest digest prevents a mutable base tag from
# changing the build underneath an unchanged rara commit.
FROM --platform=linux/amd64 oven/bun:1.3.14@sha256:e10577f0db68676a7024391c6e5cb4b879ebd17188ab750cf10024a6d700e5c4 AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates \
        git \
    && rm -rf /var/lib/apt/lists/*

# bun.lock records the public style package with a git+ssh URL. Container
# builds do not carry developer SSH credentials, so fetch public GitHub
# dependencies over HTTPS instead.
RUN git config --global url."https://github.com/".insteadOf "ssh://git@github.com/"

WORKDIR /build/web
COPY web/package.json web/bun.lock ./
RUN --mount=type=cache,target=/root/.bun/install/cache \
    bun install --frozen-lockfile

COPY web/ ./
RUN bun run build

# The unprivileged image listens above the privileged-port range and writes
# its pid and temporary files below /tmp, so it can run with a read-only root
# filesystem plus a writable /tmp emptyDir in Kubernetes.
FROM --platform=linux/amd64 nginxinc/nginx-unprivileged:1.29.4-alpine@sha256:a6c4f61f456b85b8fdf7ec7ab28cc3e299440e6fb4a9dea520e5fd8fd440025e AS runtime

COPY --from=builder --chown=101:101 /build/web/dist/ /usr/share/nginx/html/
COPY --chown=101:101 docker/nginx.conf /etc/nginx/conf.d/default.conf

USER 101:101
EXPOSE 8080
