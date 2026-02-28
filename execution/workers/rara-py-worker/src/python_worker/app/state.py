# Copyright 2025 Crrow
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
