"""Shared process state used by HTTP probes and server startup flow."""

from __future__ import annotations

from dataclasses import dataclass, field

from python_worker.core.registry import CapabilityRegistry
from python_worker.core.task_store import InMemoryTaskStore


@dataclass(slots=True)
class WorkerState:
    """Mutable readiness/liveness state shared across transports."""

    registry: CapabilityRegistry
    task_store: InMemoryTaskStore = field(default_factory=InMemoryTaskStore)
    initialized: bool = False
    require_grpc_ready: bool = False
    grpc_ready: bool = False

    def mark_initialized(self) -> None:
        """Mark application bootstrap as complete."""
        self.initialized = True

    def mark_grpc_ready(self) -> None:
        """Mark the gRPC server as started and accepting connections."""
        self.grpc_ready = True

    def mark_grpc_not_ready(self) -> None:
        """Mark the gRPC server as unavailable."""
        self.grpc_ready = False

    def liveness(self) -> tuple[bool, dict[str, object]]:
        """Return process liveness payload (no downstream dependency checks)."""
        return True, {"ok": True}

    def readiness(self) -> tuple[bool, dict[str, object]]:
        """Return readiness payload combining init, capability, and gRPC state."""
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
