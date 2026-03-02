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

"""Application container assembly for the Python worker process."""

from __future__ import annotations

import os
from dataclasses import dataclass

from python_worker.app.state import WorkerState
from python_worker.capabilities.jobspy import jobspy_scrape
from python_worker.capabilities.mem0 import (
    mem0_add,
    mem0_delete,
    mem0_delete_all,
    mem0_from_config,
    mem0_get,
    mem0_get_all,
    mem0_history,
    mem0_reset,
    mem0_search,
    mem0_update,
)
from python_worker.capabilities.system import health_ping, system_echo
from python_worker.core.executor import CapabilityExecutor
from python_worker.core.registry import CapabilityRegistry
from python_worker.core.task_store import InMemoryTaskStore
from python_worker.core.worker_identity import WorkerIdentity


@dataclass(slots=True)
class WorkerContainer:
    """Wires together long-lived app components for process startup."""

    state: WorkerState
    executor: CapabilityExecutor
    identity: WorkerIdentity


def build_container() -> WorkerContainer:
    """Build the default runtime container used by HTTP/gRPC entrypoints."""
    registry = CapabilityRegistry()
    registry.register("system.health.ping", health_ping)
    registry.register("system.echo", system_echo)
    registry.register("jobspy.scrape_jobs", jobspy_scrape)
    registry.register("mem0.from_config", mem0_from_config)
    registry.register("mem0.add", mem0_add)
    registry.register("mem0.get_all", mem0_get_all)
    registry.register("mem0.get", mem0_get)
    registry.register("mem0.search", mem0_search)
    registry.register("mem0.update", mem0_update)
    registry.register("mem0.history", mem0_history)
    registry.register("mem0.delete", mem0_delete)
    registry.register("mem0.delete_all", mem0_delete_all)
    registry.register("mem0.reset", mem0_reset)

    task_store = InMemoryTaskStore()
    state = WorkerState(
        registry=registry,
        task_store=task_store,
        require_grpc_ready=True,
    )
    state.mark_initialized()

    executor = CapabilityExecutor(registry=registry, task_store=task_store)
    identity = WorkerIdentity(
        name=os.getenv("RARA_WORKER_NAME", "rara-py-worker"),
        kind="python",
    )
    return WorkerContainer(state=state, executor=executor, identity=identity)
