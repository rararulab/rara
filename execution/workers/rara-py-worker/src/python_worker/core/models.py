from __future__ import annotations

from dataclasses import dataclass
from time import monotonic


@dataclass(slots=True)
class WorkerError:
    code: str
    message: str
    retryable: bool = False
    details: dict | None = None


@dataclass(slots=True)
class InvokeSuccess:
    capability: str
    result: dict
    duration_ms: int


@dataclass(slots=True)
class InvokeOutcome:
    success: InvokeSuccess | None = None
    error: WorkerError | None = None


@dataclass(slots=True)
class TaskRef:
    id: str
    status: str


@dataclass(slots=True)
class SubmitTaskOutcome:
    task: TaskRef | None = None
    error: WorkerError | None = None


def elapsed_ms(start: float) -> int:
    return int((monotonic() - start) * 1000)

