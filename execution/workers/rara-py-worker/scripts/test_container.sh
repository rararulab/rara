#!/usr/bin/env bash
# Build and smoke-test the rara-py-worker container over HTTP, gRPC, and grpcurl reflection.
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
WORKER_DIR="$(cd -- "${SCRIPT_DIR}/.." && pwd)"
REPO_ROOT="$(cd -- "${WORKER_DIR}/../../.." && pwd)"

DEFAULT_PYTHON_VERSION="$(cat "${WORKER_DIR}/.python-version")"
TAG="${1:-rara-py-worker:${DEFAULT_PYTHON_VERSION}}"
WORKER_NAME="${2:-smoke-worker}"
HTTP_PORT="${3:-18080}"
GRPC_PORT="${4:-15051}"
CONTAINER_NAME="rara-py-worker-smoke-$RANDOM"

export NO_PROXY="127.0.0.1,localhost"
export no_proxy="${NO_PROXY}"
unset HTTP_PROXY HTTPS_PROXY ALL_PROXY http_proxy https_proxy all_proxy

grpcurl_cmd() {
  if command -v grpcurl >/dev/null 2>&1; then
    grpcurl "$@"
    return
  fi

  # Fallback for environments without local grpcurl (e.g. fresh dev machines).
  docker run --rm fullstorydev/grpcurl:latest "$@"
}

grpcurl_target() {
  if command -v grpcurl >/dev/null 2>&1; then
    printf '127.0.0.1:%s' "${GRPC_PORT}"
    return
  fi

  # grpcurl fallback runs inside a container; use host.docker.internal to reach host ports.
  printf 'host.docker.internal:%s' "${GRPC_PORT}"
}

cleanup() {
  docker rm -f "${CONTAINER_NAME}" >/dev/null 2>&1 || true
}
trap cleanup EXIT

echo "🐳 Building image ${TAG}..."
(cd "${WORKER_DIR}" && just build-image "${TAG}")

echo "🚀 Starting container ${CONTAINER_NAME}..."
docker run -d --rm \
  --name "${CONTAINER_NAME}" \
  -e "RARA_WORKER_NAME=${WORKER_NAME}" \
  -p "${HTTP_PORT}:8080" \
  -p "${GRPC_PORT}:50051" \
  "${TAG}" >/dev/null

echo "⏳ Waiting for /healthz and /readyz..."
for i in $(seq 1 60); do
  if curl -fsS "http://127.0.0.1:${HTTP_PORT}/healthz" >/dev/null \
    && curl -fsS "http://127.0.0.1:${HTTP_PORT}/readyz" >/dev/null; then
    break
  fi
  if [[ "${i}" == "60" ]]; then
    echo "❌ worker probes did not become ready"
    docker logs "${CONTAINER_NAME}" || true
    exit 1
  fi
  sleep 1
done
echo "✅ HTTP probes are ready"

echo "🔎 Verifying gRPC Status..."
(
  cd "${WORKER_DIR}"
  PYTHON_WORKER_REPO_ROOT="${REPO_ROOT}" \
    uv run --dev python scripts/check_grpc_status.py "${GRPC_PORT}" "${WORKER_NAME}"
)

echo "🔎 Verifying gRPC reflection via grpcurl..."
GRPCURL_TARGET="$(grpcurl_target)"
SERVICES="$(grpcurl_cmd -plaintext "${GRPCURL_TARGET}" list)"
echo "${SERVICES}" | grep -q '^execution\.v1\.ExecutionWorkerService$'
METHODS="$(grpcurl_cmd -plaintext "${GRPCURL_TARGET}" list execution.v1.ExecutionWorkerService)"
echo "${METHODS}" | grep -q '^execution\.v1\.ExecutionWorkerService\.Status$'
echo "${METHODS}" | grep -q '^execution\.v1\.ExecutionWorkerService\.Invoke$'
echo "${METHODS}" | grep -q '^execution\.v1\.ExecutionWorkerService\.SubmitTask$'
echo "${METHODS}" | grep -q '^execution\.v1\.ExecutionWorkerService\.GetTask$'
echo "grpcurl reflection OK"

echo "✅ Container smoke test passed"
