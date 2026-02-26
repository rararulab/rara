# Rara Py Worker

Kubernetes execution worker implementation in Python.

## Scope (v1)

- HTTP probe endpoints: `/healthz`, `/readyz`
- Capability registry abstraction
- In-memory task store (async task state)
- gRPC server wired to execution core (expects generated stubs from `api/proto/execution/v1/worker.proto`)
- Example capabilities: `system.health.ping`, `system.echo`

## Environment (uv)

Use `uv` to manage the project environment and dependencies.

```bash
cd /Users/ryan/code/personal/rust/job/execution/workers/rara-py-worker
just sync
```

## Generate gRPC stubs

```bash
cd /Users/ryan/code/personal/rust/job/api
buf generate
```

This generates Python execution stubs under `/Users/ryan/code/personal/rust/job/api-generated/python/pb`.

## Local run (combined HTTP probes + gRPC)

```bash
cd /Users/ryan/code/personal/rust/job
uv run --project execution/workers/rara-py-worker python -m python_worker.app.server
```

Then:

- `GET http://127.0.0.1:8080/healthz`
- `GET http://127.0.0.1:8080/readyz`
- gRPC on `127.0.0.1:50051`
- `ExecutionWorkerService.Status` returns worker identity (`name`, `kind`)

Set a dynamic worker name at runtime (useful when spawning workers on demand):

```bash
RARA_WORKER_NAME=search-worker-42 \
uv run --project execution/workers/rara-py-worker python -m python_worker.app.server
```

## Build image

Preferred (uses `/Users/ryan/code/personal/rust/job/execution/workers/rara-py-worker/.python-version` as the source of truth):

```bash
cd /Users/ryan/code/personal/rust/job/execution/workers/rara-py-worker
just build-image
```

```bash
cd /Users/ryan/code/personal/rust/job
docker build -f execution/workers/rara-py-worker/Dockerfile -t rara-py-worker:3.13 .
```

Override Python version at build time if needed:

```bash
cd /Users/ryan/code/personal/rust/job
docker build \
  --build-arg PYTHON_VERSION=3.12 \
  -f execution/workers/rara-py-worker/Dockerfile \
  -t rara-py-worker:3.12 .
```

Or via local `justfile`:

```bash
cd /Users/ryan/code/personal/rust/job/execution/workers/rara-py-worker
just build-image-python 3.12
```

## gRPC server only

```bash
cd /Users/ryan/code/personal/rust/job
uv run --project execution/workers/rara-py-worker \
  python -m python_worker.app.grpc_server
```

## Tests

```bash
cd /Users/ryan/code/personal/rust/job/execution/workers/rara-py-worker
just test
```

## Notes

- Dependencies are declared in `pyproject.toml` and installed via `uv sync`.
- `uv.lock` should be checked in once generated to keep environments reproducible.
- `readyz` requires gRPC startup when launched via `python_worker.app.server` (shared `WorkerState`).
- Rust proto code generation remains in `/Users/ryan/code/personal/rust/job/api/build.rs` (tonic/prost), not Buf.
- `.python-version` is the source of truth for local `uv` usage and the default Docker build Python version/tag via `execution/workers/rara-py-worker/justfile`.
- Docker image installs Python via `uv python install`, not via `python:*` base image.
- OpenTelemetry tracing is initialized with a stdout exporter (`ConsoleSpanExporter`) so logs/spans can be collected by the Kubernetes logging pipeline.
- Worker identity:
  - `name`: from `RARA_WORKER_NAME` (default `rara-py-worker`)
  - `kind`: fixed to `python` for this implementation
