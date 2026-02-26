from __future__ import annotations

import sys
import time

import grpc

from python_worker.app.grpc_server import load_generated_modules


def main() -> int:
    if len(sys.argv) != 3:
        raise SystemExit("usage: check_grpc_status.py <grpc_port> <expected_name>")

    grpc_port = int(sys.argv[1])
    expected_name = sys.argv[2]
    modules = load_generated_modules()

    channel = grpc.insecure_channel(f"127.0.0.1:{grpc_port}")
    stub = modules.worker_pb2_grpc.ExecutionWorkerServiceStub(channel)

    deadline = time.time() + 20
    last_err: Exception | None = None
    while time.time() < deadline:
        try:
            resp = stub.Status(modules.worker_pb2.StatusRequest(), timeout=2)
            if not resp.HasField("success"):
                raise RuntimeError("Status response missing success outcome")
            if resp.success.worker.name != expected_name:
                raise RuntimeError(
                    f"unexpected worker name: {resp.success.worker.name} != {expected_name}"
                )
            if resp.success.worker.kind != modules.worker_pb2.WORKER_KIND_PYTHON:
                raise RuntimeError(f"unexpected worker kind: {resp.success.worker.kind}")
            print("gRPC Status OK")
            return 0
        except Exception as exc:  # noqa: BLE001
            last_err = exc
            time.sleep(0.5)

    raise SystemExit(f"gRPC Status failed: {last_err}")


if __name__ == "__main__":
    raise SystemExit(main())
