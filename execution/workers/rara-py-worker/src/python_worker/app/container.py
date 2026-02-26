"""Application container assembly for the Python worker process."""

from __future__ import annotations

import os
from dataclasses import dataclass

from python_worker.app.state import WorkerState
from python_worker.capabilities.jobspy import jobspy_scrape
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
