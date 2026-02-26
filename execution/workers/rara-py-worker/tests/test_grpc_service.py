import asyncio

import pytest
from google.protobuf.struct_pb2 import Struct

from python_worker.app.grpc_server import (
    ExecutionWorkerGrpcService,
    GrpcServerBootstrapError,
    load_generated_modules,
)
from python_worker.core.executor import CapabilityExecutor
from python_worker.core.registry import CapabilityRegistry
from python_worker.core.worker_identity import WorkerIdentity


def _make_struct(data: dict) -> Struct:
    msg = Struct()
    msg.update(data)
    return msg


def test_grpc_invoke_maps_success_response() -> None:
    try:
        modules = load_generated_modules()
    except GrpcServerBootstrapError as exc:
        pytest.skip(str(exc))
    registry = CapabilityRegistry()
    registry.register("system.echo", lambda payload: {"echo": payload["message"]})
    service = ExecutionWorkerGrpcService(
        modules=modules,
        executor=CapabilityExecutor(registry=registry),
        identity=WorkerIdentity(name="test-worker", kind="python"),
    )
    request = modules.worker_pb2.InvokeRequest(
        capability="system.echo",
        payload=_make_struct({"message": "hello"}),
    )

    response = asyncio.run(service.Invoke(request, None))

    assert response.HasField("success")
    assert response.success.capability == "system.echo"
    assert dict(response.success.result) == {"echo": "hello"}


def test_grpc_status_reports_worker_identity() -> None:
    try:
        modules = load_generated_modules()
    except GrpcServerBootstrapError as exc:
        pytest.skip(str(exc))

    service = ExecutionWorkerGrpcService(
        modules=modules,
        executor=CapabilityExecutor(registry=CapabilityRegistry()),
        identity=WorkerIdentity(name="dynamic-worker", kind="python"),
    )

    response = asyncio.run(service.Status(modules.worker_pb2.StatusRequest(), None))

    assert response.HasField("success")
    assert response.success.worker.name == "dynamic-worker"
    assert response.success.worker.kind == modules.worker_pb2.WORKER_KIND_PYTHON


def test_grpc_submit_and_get_task_round_trip() -> None:
    try:
        modules = load_generated_modules()
    except GrpcServerBootstrapError as exc:
        pytest.skip(str(exc))
    registry = CapabilityRegistry()

    async def echo(payload: dict) -> dict:
        await asyncio.sleep(0.01)
        return {"echo": payload["message"]}

    registry.register("system.echo", echo)
    service = ExecutionWorkerGrpcService(
        modules=modules,
        executor=CapabilityExecutor(registry=registry),
        identity=WorkerIdentity(name="test-worker", kind="python"),
    )

    async def scenario() -> None:
        submit = await service.SubmitTask(
            modules.worker_pb2.SubmitTaskRequest(
                capability="system.echo",
                payload=_make_struct({"message": "hi"}),
            ),
            None,
        )
        assert submit.HasField("task")

        task_id = submit.task.id
        for _ in range(20):
            status = await service.GetTask(
                modules.worker_pb2.GetTaskRequest(task_id=task_id),
                None,
            )
            if status.HasField("task") and (
                status.task.status == modules.worker_pb2.TASK_STATE_SUCCEEDED
            ):
                assert dict(status.task.result) == {"echo": "hi"}
                return
            await asyncio.sleep(0.01)
        raise AssertionError("task did not reach succeeded state")

    asyncio.run(scenario())
