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

"""Combined process entrypoint that starts HTTP probes and gRPC server."""

from __future__ import annotations

import asyncio
import os

import uvicorn

from python_worker.app.container import build_container
from python_worker.app.grpc_server import serve as serve_grpc
from python_worker.app.http import create_app
from python_worker.observability.tracing import init_tracing


async def serve() -> None:
    """Run both HTTP and gRPC servers with shared state/executor."""
    init_tracing()
    container = build_container()
    http_app = create_app(state=container.state)

    http_host = os.getenv("PYTHON_WORKER_HTTP_HOST", "0.0.0.0")
    http_port = int(os.getenv("PYTHON_WORKER_HTTP_PORT", "8080"))
    grpc_host = os.getenv("PYTHON_WORKER_GRPC_HOST", "0.0.0.0")
    grpc_port = int(os.getenv("PYTHON_WORKER_GRPC_PORT", "50051"))

    uvicorn_config = uvicorn.Config(http_app, host=http_host, port=http_port, log_level="info")
    uvicorn_server = uvicorn.Server(uvicorn_config)

    await asyncio.gather(
        uvicorn_server.serve(),
        serve_grpc(
            executor=container.executor,
            identity=container.identity,
            state=container.state,
            host=grpc_host,
            port=grpc_port,
        ),
    )


def main() -> None:
    """CLI entrypoint for the combined worker server."""
    asyncio.run(serve())


if __name__ == "__main__":
    main()
