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
