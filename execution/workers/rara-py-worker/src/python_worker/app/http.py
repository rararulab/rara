from __future__ import annotations

from contextlib import asynccontextmanager

from fastapi import FastAPI, Response, status
from starlette.requests import Request

from python_worker.app.state import WorkerState
from python_worker.core.registry import CapabilityRegistry
from python_worker.observability.tracing import get_tracer


def build_default_state() -> WorkerState:
    registry = CapabilityRegistry()
    # Placeholder registration to make readiness behavior explicit in local runs.
    registry.register("system.health.ping", object())
    state = WorkerState(registry=registry)
    state.mark_initialized()
    state.mark_grpc_ready()
    return state


def create_app(state: WorkerState | None = None) -> FastAPI:
    worker_state = state or build_default_state()
    tracer = get_tracer("python_worker.http")

    @asynccontextmanager
    async def lifespan(_app: FastAPI):
        yield

    app = FastAPI(title="rara-py-worker", lifespan=lifespan)
    app.state.worker_state = worker_state

    @app.middleware("http")
    async def trace_requests(request: Request, call_next):
        with tracer.start_as_current_span(
            f"http {request.method} {request.url.path}"
        ) as span:
            span.set_attribute("http.method", request.method)
            span.set_attribute("http.route", request.url.path)
            response = await call_next(request)
            span.set_attribute("http.status_code", response.status_code)
            return response

    @app.get("/healthz")
    async def healthz(response: Response) -> dict[str, object]:
        ok, payload = app.state.worker_state.liveness()
        response.status_code = status.HTTP_200_OK if ok else status.HTTP_503_SERVICE_UNAVAILABLE
        return payload

    @app.get("/readyz")
    async def readyz(response: Response) -> dict[str, object]:
        ok, payload = app.state.worker_state.readiness()
        response.status_code = status.HTTP_200_OK if ok else status.HTTP_503_SERVICE_UNAVAILABLE
        return payload

    return app


def main() -> None:
    import uvicorn

    uvicorn.run(create_app(), host="0.0.0.0", port=8080)


if __name__ == "__main__":
    main()
