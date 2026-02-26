from __future__ import annotations

from dataclasses import dataclass, field

from python_worker.core.registry import CapabilityRegistry
from python_worker.core.task_store import InMemoryTaskStore


@dataclass(slots=True)
class WorkerState:
    registry: CapabilityRegistry
    task_store: InMemoryTaskStore = field(default_factory=InMemoryTaskStore)
    initialized: bool = False
    require_grpc_ready: bool = False
    grpc_ready: bool = False

    def mark_initialized(self) -> None:
        self.initialized = True

    def mark_grpc_ready(self) -> None:
        self.grpc_ready = True

    def mark_grpc_not_ready(self) -> None:
        self.grpc_ready = False

    def liveness(self) -> tuple[bool, dict[str, object]]:
        return True, {"ok": True}

    def readiness(self) -> tuple[bool, dict[str, object]]:
        capability_count = self.registry.count()
        grpc_ready = (not self.require_grpc_ready) or self.grpc_ready
        ready = self.initialized and capability_count > 0 and grpc_ready
        return (
            ready,
            {
                "ready": ready,
                "initialized": self.initialized,
                "grpc_ready": self.grpc_ready,
                "require_grpc_ready": self.require_grpc_ready,
                "capability_count": capability_count,
            },
        )
