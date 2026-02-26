from __future__ import annotations

import asyncio
import os
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any

import grpc
from google.protobuf import json_format
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
    worker_pb2: Any
    worker_pb2_grpc: Any


def build_default_executor() -> CapabilityExecutor:
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

    async def Invoke(self, request, context):  # noqa: N802 (grpc generated naming)
        with self._tracer.start_as_current_span("grpc.Invoke") as span:
            span.set_attribute("rpc.system", "grpc")
            span.set_attribute("rpc.method", "Invoke")
            span.set_attribute("capability", request.capability)
            payload = json_format.MessageToDict(
                request.payload, preserving_proto_field_name=True
            )
            outcome = await self._executor.invoke(request.capability, payload)
            if outcome.success is not None:
                span.set_attribute("result.status", "success")
                result_struct = self._struct_from_dict(outcome.success.result)
                return self._pb2.InvokeResponse(
                    success=self._pb2.InvokeSuccess(
                        capability=outcome.success.capability,
                        result=result_struct,
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
        with self._tracer.start_as_current_span("grpc.SubmitTask") as span:
            span.set_attribute("rpc.system", "grpc")
            span.set_attribute("rpc.method", "SubmitTask")
            span.set_attribute("capability", request.capability)
            payload = json_format.MessageToDict(
                request.payload, preserving_proto_field_name=True
            )
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
            msg.result.CopyFrom(self._struct_from_dict(task.result))
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
    def _struct_from_dict(data: dict[str, Any]):
        from google.protobuf.struct_pb2 import Struct

        msg = Struct()
        json_format.ParseDict(data, msg)
        return msg

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
    modules = load_generated_modules()
    exec_impl = executor or build_default_executor()
    service_impl = ExecutionWorkerGrpcService(
        modules=modules,
        executor=exec_impl,
        identity=identity or WorkerIdentity(name="rara-py-worker", kind="python"),
    )

    server = grpc.aio.server()
    modules.worker_pb2_grpc.add_ExecutionWorkerServiceServicer_to_server(
        service_impl, server
    )
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
    asyncio.run(serve())


if __name__ == "__main__":
    main()
