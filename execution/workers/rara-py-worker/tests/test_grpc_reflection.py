import asyncio
import os

import grpc
import pytest
from google.protobuf import descriptor_pb2
from grpc_reflection.v1alpha import reflection_pb2, reflection_pb2_grpc

from python_worker.app.grpc_server import (
    GrpcServerBootstrapError,
    create_grpc_server,
)
from python_worker.core.executor import CapabilityExecutor
from python_worker.core.registry import CapabilityRegistry
from python_worker.core.worker_identity import WorkerIdentity


def test_grpc_reflection_lists_execution_worker_methods() -> None:
    try:
        asyncio.run(_assert_reflection_methods())
    except GrpcServerBootstrapError as exc:
        pytest.skip(str(exc))


async def _assert_reflection_methods() -> None:
    server = await create_grpc_server(
        executor=CapabilityExecutor(registry=CapabilityRegistry()),
        identity=WorkerIdentity(name="reflection-test", kind="python"),
    )
    port = server.add_insecure_port("127.0.0.1:0")
    await server.start()
    try:
        service_names, method_names = await _fetch_reflection_data(port)
        assert "execution.v1.ExecutionWorkerService" in service_names
        assert "grpc.reflection.v1alpha.ServerReflection" in service_names
        assert method_names == ["Status", "ListCapabilities", "Invoke", "SubmitTask", "GetTask"]
    finally:
        await server.stop(grace=None)


async def _fetch_reflection_data(port: int) -> tuple[list[str], list[str]]:
    for key in ("HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "http_proxy", "https_proxy", "all_proxy"):
        os.environ.pop(key, None)
    os.environ["NO_PROXY"] = "127.0.0.1,localhost"
    os.environ["no_proxy"] = "127.0.0.1,localhost"

    async with grpc.aio.insecure_channel(f"127.0.0.1:{port}") as channel:
        stub = reflection_pb2_grpc.ServerReflectionStub(channel)

        list_req = reflection_pb2.ServerReflectionRequest(list_services="")
        list_stream = stub.ServerReflectionInfo(_single_request_stream(list_req))
        list_resp = await list_stream.read()
        service_names = sorted(item.name for item in list_resp.list_services_response.service)

        file_req = reflection_pb2.ServerReflectionRequest(
            file_containing_symbol="execution.v1.ExecutionWorkerService"
        )
        file_stream = stub.ServerReflectionInfo(_single_request_stream(file_req))
        file_resp = await file_stream.read()

        method_names: list[str] = []
        for raw in file_resp.file_descriptor_response.file_descriptor_proto:
            fd = descriptor_pb2.FileDescriptorProto()
            fd.ParseFromString(raw)
            if fd.package != "execution.v1":
                continue
            for service in fd.service:
                if service.name == "ExecutionWorkerService":
                    method_names = [method.name for method in service.method]
                    break
        return service_names, method_names


async def _single_request_stream(request):
    yield request
