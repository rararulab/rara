# Copyright 2025 Rararulab
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#      http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

"""gRPC server bootstrap and service implementation for execution worker RPCs."""

from __future__ import annotations

import asyncio
import json
import os
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any

import grpc
from google.protobuf.timestamp_pb2 import Timestamp
from grpc_reflection.v1alpha import reflection

from python_worker.app.state import WorkerState
from python_worker.core.executor import CapabilityExecutor
from python_worker.core.registry import CapabilityRegistry
from python_worker.core.task_store import TaskRecord
from python_worker.core.worker_identity import WorkerIdentity
from python_worker.observability.tracing import get_tracer


class GrpcServerBootstrapError(RuntimeError):
    """Raised when generated protobuf stubs are unavailable."""


@dataclass(slots=True)
class GeneratedGrpcModules:
    """Holds generated protobuf/grpc modules loaded from api-generated stubs."""

    worker_pb2: Any
    worker_pb2_grpc: Any


def build_default_executor() -> CapabilityExecutor:
    """Create a minimal executor for standalone gRPC-only local runs."""
    registry = CapabilityRegistry()

    async def _health_ping(payload: dict[str, Any]) -> dict[str, Any]:
        return {"ok": True, "echo": payload}

    registry.register("system.health.ping", _health_ping)
    return CapabilityExecutor(registry=registry)


def _repo_root_from_here() -> Path:
    # Search upwards for the generated API bindings root in both repo and container layouts.
    current = Path(__file__).resolve()
    for parent in current.parents:
        if (parent / "api-generated" / "python" / "pb").exists():
            return parent
    # Container layout fallback (`/app/src/...` -> `/app`)
    if len(current.parents) >= 4:
        return current.parents[3]
    return current.parent


def _ensure_generated_python_bindings_on_path() -> Path:
    root = Path(os.getenv("PYTHON_WORKER_REPO_ROOT", _repo_root_from_here()))
    bindings_root = root / "api-generated" / "python" / "pb"
    if not bindings_root.exists():
        raise GrpcServerBootstrapError(
            f"Generated Python bindings not found at {bindings_root}. "
            "Run `cd api && buf generate` first."
        )
    path_str = str(bindings_root)
    if path_str not in sys.path:
        sys.path.insert(0, path_str)
    return bindings_root


def load_generated_modules() -> GeneratedGrpcModules:
    """Import generated execution worker protobuf modules from `api-generated`."""
    _ensure_generated_python_bindings_on_path()
    try:
        from execution.v1 import worker_pb2, worker_pb2_grpc  # type: ignore
    except ModuleNotFoundError as exc:  # pragma: no cover
        raise GrpcServerBootstrapError(
            "Unable to import generated execution worker protobuf modules. "
            "Run `cd api && buf generate` and ensure `api-generated/python/pb` is present."
        ) from exc
    return GeneratedGrpcModules(worker_pb2=worker_pb2, worker_pb2_grpc=worker_pb2_grpc)


class ExecutionWorkerGrpcService:
    """Implements the generated gRPC servicer methods for worker RPCs."""

    def __init__(
        self,
        modules: GeneratedGrpcModules,
        executor: CapabilityExecutor,
        identity: WorkerIdentity,
    ) -> None:
        self._pb2 = modules.worker_pb2
        self._executor = executor
        self._identity = identity
        self._tracer = get_tracer("python_worker.grpc")

    async def Status(self, request, context):  # noqa: N802
        """Return worker identity metadata for discovery/diagnostics."""
        with self._tracer.start_as_current_span("grpc.Status") as span:
            span.set_attribute("rpc.system", "grpc")
            span.set_attribute("rpc.method", "Status")
            span.set_attribute("worker.name", self._identity.name)
            span.set_attribute("worker.kind", self._identity.kind)
            return self._pb2.StatusResponse(
                success=self._pb2.StatusSuccess(
                    worker=self._pb2.WorkerIdentity(
                        name=self._identity.name,
                        kind=self._to_proto_worker_kind(self._identity.kind),
                    )
                )
            )

    async def ListCapabilities(self, request, context):  # noqa: N802
        """Return the set of capability names available on this worker instance."""
        with self._tracer.start_as_current_span("grpc.ListCapabilities") as span:
            span.set_attribute("rpc.system", "grpc")
            span.set_attribute("rpc.method", "ListCapabilities")
            names = self._executor.list_capabilities()
            span.set_attribute("capability.count", len(names))
            return self._pb2.ListCapabilitiesResponse(
                success=self._pb2.ListCapabilitiesSuccess(
                    capabilities=[
                        self._pb2.CapabilityInfo(
                            name=name,
                            supports_sync=True,
                            supports_async=True,
                        )
                        for name in names
                    ]
                )
            )

    async def Invoke(self, request, context):  # noqa: N802 (grpc generated naming)
        """Handle synchronous capability invocation requests."""
        with self._tracer.start_as_current_span("grpc.Invoke") as span:
            span.set_attribute("rpc.system", "grpc")
            span.set_attribute("rpc.method", "Invoke")
            span.set_attribute("capability", request.capability)
            payload = self._dict_from_json_bytes(request.payload)
            outcome = await self._executor.invoke(request.capability, payload)
            if outcome.success is not None:
                span.set_attribute("result.status", "success")
                return self._pb2.InvokeResponse(
                    success=self._pb2.InvokeSuccess(
                        capability=outcome.success.capability,
                        result=self._json_bytes_from_dict(outcome.success.result),
                        duration_ms=outcome.success.duration_ms,
                    )
                )
            err = outcome.error
            span.set_attribute("result.status", "error")
            return self._pb2.InvokeResponse(
                error=self._pb2.WorkerError(
                    code=err.code if err else "INTERNAL_ERROR",
                    message=err.message if err else "Unknown error",
                    retryable=bool(err.retryable) if err else False,
                )
            )

    async def SubmitTask(self, request, context):  # noqa: N802
        """Handle asynchronous capability submission requests."""
        with self._tracer.start_as_current_span("grpc.SubmitTask") as span:
            span.set_attribute("rpc.system", "grpc")
            span.set_attribute("rpc.method", "SubmitTask")
            span.set_attribute("capability", request.capability)
            payload = self._dict_from_json_bytes(request.payload)
            outcome = await self._executor.submit_task(request.capability, payload)
            if outcome.task is not None:
                span.set_attribute("task.id", outcome.task.id)
                span.set_attribute("result.status", "queued")
                return self._pb2.SubmitTaskResponse(
                    task=self._pb2.TaskRef(
                        id=outcome.task.id,
                        status=self._pb2.TASK_STATE_QUEUED,
                    )
                )
            err = outcome.error
            span.set_attribute("result.status", "error")
            return self._pb2.SubmitTaskResponse(
                error=self._pb2.WorkerError(
                    code=err.code if err else "INTERNAL_ERROR",
                    message=err.message if err else "Unknown error",
                    retryable=bool(err.retryable) if err else False,
                )
            )

    async def GetTask(self, request, context):  # noqa: N802
        """Return current task status/result for a submitted task."""
        with self._tracer.start_as_current_span("grpc.GetTask") as span:
            span.set_attribute("rpc.system", "grpc")
            span.set_attribute("rpc.method", "GetTask")
            span.set_attribute("task.id", request.task_id)
            task = self._executor.get_task(request.task_id)
            if task is None:
                span.set_attribute("result.status", "not_found")
                return self._pb2.GetTaskResponse(
                    error=self._pb2.WorkerError(
                        code="TASK_NOT_FOUND",
                        message=f"Task not found: {request.task_id}",
                        retryable=False,
                    )
                )

            span.set_attribute("result.status", task.status.value)
            response = self._pb2.GetTaskResponse(task=self._task_status_message(task))
            return response

    def _task_status_message(self, task: TaskRecord):
        msg = self._pb2.TaskStatus(
            id=task.id,
            capability=task.capability,
            status=self._to_proto_task_state(task.status.value),
            created_at=self._timestamp(task.created_at),
        )
        if task.started_at is not None:
            msg.started_at.CopyFrom(self._timestamp(task.started_at))
        if task.finished_at is not None:
            msg.finished_at.CopyFrom(self._timestamp(task.finished_at))
        if task.result is not None:
            msg.result = self._json_bytes_from_dict(task.result)
        if task.error is not None:
            msg.error.CopyFrom(
                self._pb2.WorkerError(
                    code=str(task.error.get("code", "CAPABILITY_EXECUTION_FAILED")),
                    message=str(task.error.get("message", "Task failed")),
                    retryable=bool(task.error.get("retryable", False)),
                )
            )
        return msg

    @staticmethod
    def _timestamp(dt) -> Timestamp:
        pb = Timestamp()
        pb.FromDatetime(dt)
        return pb

    @staticmethod
    def _json_bytes_from_dict(data: dict[str, Any]) -> bytes:
        return json.dumps(data, separators=(",", ":"), ensure_ascii=False).encode("utf-8")

    @staticmethod
    def _dict_from_json_bytes(data: bytes) -> dict[str, Any]:
        if not data:
            return {}
        parsed = json.loads(data.decode("utf-8"))
        if not isinstance(parsed, dict):
            raise ValueError("execution payload must decode to a JSON object")
        return parsed

    def _to_proto_task_state(self, value: str) -> int:
        mapping = {
            "queued": self._pb2.TASK_STATE_QUEUED,
            "running": self._pb2.TASK_STATE_RUNNING,
            "succeeded": self._pb2.TASK_STATE_SUCCEEDED,
            "failed": self._pb2.TASK_STATE_FAILED,
            "canceled": self._pb2.TASK_STATE_CANCELED,
        }
        return mapping.get(value, self._pb2.TASK_STATE_UNSPECIFIED)

    def _to_proto_worker_kind(self, value: str) -> int:
        mapping = {
            "python": self._pb2.WORKER_KIND_PYTHON,
            "go": self._pb2.WORKER_KIND_GO,
        }
        return mapping.get(value, self._pb2.WORKER_KIND_UNSPECIFIED)


async def create_grpc_server(
    executor: CapabilityExecutor | None = None,
    identity: WorkerIdentity | None = None,
    state: WorkerState | None = None,
):
    """Create and configure the gRPC aio server with reflection enabled."""
    modules = load_generated_modules()
    exec_impl = executor or build_default_executor()
    service_impl = ExecutionWorkerGrpcService(
        modules=modules,
        executor=exec_impl,
        identity=identity or WorkerIdentity(name="rara-py-worker", kind="python"),
    )

    server = grpc.aio.server()
    modules.worker_pb2_grpc.add_ExecutionWorkerServiceServicer_to_server(service_impl, server)
    service_names = (
        modules.worker_pb2.DESCRIPTOR.services_by_name["ExecutionWorkerService"].full_name,
        reflection.SERVICE_NAME,
    )
    reflection.enable_server_reflection(service_names, server)

    if state is not None:
        state.mark_grpc_not_ready()

    return server


async def serve(
    executor: CapabilityExecutor | None = None,
    identity: WorkerIdentity | None = None,
    state: WorkerState | None = None,
    host: str = "0.0.0.0",
    port: int = 50051,
) -> None:
    """Run the gRPC server and keep readiness state in sync."""
    server = await create_grpc_server(executor=executor, identity=identity, state=state)
    server.add_insecure_port(f"{host}:{port}")
    await server.start()
    if state is not None:
        state.mark_grpc_ready()
    try:
        await server.wait_for_termination()
    finally:
        if state is not None:
            state.mark_grpc_not_ready()


def main() -> None:
    """CLI entrypoint for gRPC-only worker mode."""
    asyncio.run(serve())


if __name__ == "__main__":
    main()
