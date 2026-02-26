from __future__ import annotations

import asyncio
import inspect
import uuid
from datetime import UTC, datetime
from time import monotonic
from typing import Any

from python_worker.core.models import (
    InvokeOutcome,
    InvokeSuccess,
    SubmitTaskOutcome,
    TaskRef,
    WorkerError,
    elapsed_ms,
)
from python_worker.core.registry import CapabilityRegistry
from python_worker.core.task_store import InMemoryTaskStore, TaskRecord, TaskState
from python_worker.observability.tracing import get_tracer


class CapabilityExecutor:
    def __init__(
        self,
        registry: CapabilityRegistry,
        task_store: InMemoryTaskStore | None = None,
    ) -> None:
        self._registry = registry
        self._task_store = task_store or InMemoryTaskStore()
        self._tracer = get_tracer("python_worker.executor")

    async def invoke(self, capability: str, payload: dict[str, Any]) -> InvokeOutcome:
        with self._tracer.start_as_current_span("capability.invoke") as span:
            span.set_attribute("capability", capability)
            handler = self._registry.get(capability)
            if handler is None:
                span.set_attribute("result.status", "unknown_capability")
                return InvokeOutcome(
                    error=WorkerError(
                        code="UNKNOWN_CAPABILITY",
                        message=f"Capability not found: {capability}",
                        retryable=False,
                    )
                )

            started = monotonic()
            try:
                result = handler(payload)
                if inspect.isawaitable(result):
                    result = await result
                if not isinstance(result, dict):
                    result = {"value": result}
                duration_ms = elapsed_ms(started)
                span.set_attribute("result.status", "success")
                span.set_attribute("duration_ms", duration_ms)
                return InvokeOutcome(
                    success=InvokeSuccess(
                        capability=capability,
                        result=result,
                        duration_ms=duration_ms,
                    )
                )
            except Exception as exc:  # pragma: no cover - exercised by future tests
                span.record_exception(exc)
                span.set_attribute("result.status", "error")
                return InvokeOutcome(
                    error=WorkerError(
                        code="CAPABILITY_EXECUTION_FAILED",
                        message=str(exc),
                        retryable=True,
                    )
                )

    async def submit_task(
        self, capability: str, payload: dict[str, Any]
    ) -> SubmitTaskOutcome:
        with self._tracer.start_as_current_span("capability.submit_task") as span:
            span.set_attribute("capability", capability)
            task_id = f"task_{uuid.uuid4().hex[:12]}"
            task = TaskRecord(id=task_id, capability=capability, status=TaskState.QUEUED)
            self._task_store.put(task)
            span.set_attribute("task.id", task_id)

            asyncio.create_task(self._run_task(task_id, payload))

            return SubmitTaskOutcome(task=TaskRef(id=task_id, status=task.status.value))

    def get_task(self, task_id: str) -> TaskRecord | None:
        return self._task_store.get(task_id)

    async def _run_task(self, task_id: str, payload: dict[str, Any]) -> None:
        task = self._task_store.get(task_id)
        if task is None:
            return

        with self._tracer.start_as_current_span("capability.run_task") as span:
            span.set_attribute("task.id", task_id)
            span.set_attribute("capability", task.capability)

            task.status = TaskState.RUNNING
            task.started_at = datetime.now(UTC)
            self._task_store.update(task)

            outcome = await self.invoke(task.capability, payload)

            task = self._task_store.get(task_id)
            if task is None:
                return

            task.finished_at = datetime.now(UTC)
            if outcome.success is not None:
                task.status = TaskState.SUCCEEDED
                task.result = outcome.success.result
                task.error = None
                span.set_attribute("result.status", "succeeded")
            else:
                task.status = TaskState.FAILED
                task.result = None
                task.error = (
                    None
                    if outcome.error is None
                    else {
                        "code": outcome.error.code,
                        "message": outcome.error.message,
                        "retryable": outcome.error.retryable,
                    }
                )
                span.set_attribute("result.status", "failed")
            self._task_store.update(task)
