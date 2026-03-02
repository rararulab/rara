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

"""In-memory task records and lifecycle state for async execution."""

from __future__ import annotations

from dataclasses import dataclass, field
from datetime import UTC, datetime
from enum import StrEnum


class TaskState(StrEnum):
    QUEUED = "queued"
    RUNNING = "running"
    SUCCEEDED = "succeeded"
    FAILED = "failed"
    CANCELED = "canceled"


@dataclass(slots=True)
class TaskRecord:
    id: str
    capability: str
    status: TaskState
    created_at: datetime = field(default_factory=lambda: datetime.now(UTC))
    started_at: datetime | None = None
    finished_at: datetime | None = None
    result: dict | None = None
    error: dict | None = None


class InMemoryTaskStore:
    """Volatile task store used by the v1 worker implementation."""

    def __init__(self) -> None:
        self._tasks: dict[str, TaskRecord] = {}

    def put(self, task: TaskRecord) -> None:
        self._tasks[task.id] = task

    def update(self, task: TaskRecord) -> None:
        self._tasks[task.id] = task

    def get(self, task_id: str) -> TaskRecord | None:
        return self._tasks.get(task_id)

    def count(self) -> int:
        return len(self._tasks)
