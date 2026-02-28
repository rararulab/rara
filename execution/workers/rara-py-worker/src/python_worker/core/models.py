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

"""Small transport-agnostic models used by the execution core."""

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
    """Convert monotonic start time to integer elapsed milliseconds."""
    return int((monotonic() - start) * 1000)
