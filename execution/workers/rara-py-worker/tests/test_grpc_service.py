import asyncio
import json

import pytest

from python_worker.app.grpc_server import (
    ExecutionWorkerGrpcService,
    GrpcServerBootstrapError,
    load_generated_modules,
)
from python_worker.core.executor import CapabilityExecutor
from python_worker.core.registry import CapabilityRegistry
from python_worker.core.worker_identity import WorkerIdentity


def _make_payload(data: dict) -> bytes:
    return json.dumps(data).encode("utf-8")


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
        payload=_make_payload({"message": "hello"}),
    )

    response = asyncio.run(service.Invoke(request, None))

    assert response.HasField("success")
    assert response.success.capability == "system.echo"
    assert json.loads(response.success.result.decode("utf-8")) == {"echo": "hello"}


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


def test_grpc_list_capabilities_reports_registered_capabilities() -> None:
    try:
        modules = load_generated_modules()
    except GrpcServerBootstrapError as exc:
        pytest.skip(str(exc))

    registry = CapabilityRegistry()
    registry.register("system.echo", lambda payload: {"echo": payload})
    registry.register("jobspy.scrape_jobs", lambda payload: payload)
    service = ExecutionWorkerGrpcService(
        modules=modules,
        executor=CapabilityExecutor(registry=registry),
        identity=WorkerIdentity(name="test-worker", kind="python"),
    )

    response = asyncio.run(
        service.ListCapabilities(modules.worker_pb2.ListCapabilitiesRequest(), None)
    )

    assert response.HasField("success")
    assert [item.name for item in response.success.capabilities] == [
        "jobspy.scrape_jobs",
        "system.echo",
    ]
    assert all(item.supports_sync for item in response.success.capabilities)
    assert all(item.supports_async for item in response.success.capabilities)


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
                payload=_make_payload({"message": "hi"}),
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
                assert json.loads(status.task.result.decode("utf-8")) == {"echo": "hi"}
                return
            await asyncio.sleep(0.01)
        raise AssertionError("task did not reach succeeded state")

    asyncio.run(scenario())
