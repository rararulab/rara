import asyncio

from python_worker.core.executor import CapabilityExecutor
from python_worker.core.registry import CapabilityRegistry


async def _wait_for_task(executor: CapabilityExecutor, task_id: str, attempts: int = 20):
    for _ in range(attempts):
        status = executor.get_task(task_id)
        if status is not None and status.status in {"succeeded", "failed", "canceled"}:
            return status
        await asyncio.sleep(0.01)
    return executor.get_task(task_id)


def test_invoke_sync_success_returns_result() -> None:
    registry = CapabilityRegistry()
    registry.register(
        "system.echo",
        lambda payload: {"echo": payload["message"]},
    )
    executor = CapabilityExecutor(registry=registry)

    outcome = asyncio.run(executor.invoke("system.echo", {"message": "hi"}))

    assert outcome.success is not None
    assert outcome.success.capability == "system.echo"
    assert outcome.success.result == {"echo": "hi"}
    assert outcome.error is None


def test_invoke_unknown_capability_returns_error() -> None:
    executor = CapabilityExecutor(registry=CapabilityRegistry())

    outcome = asyncio.run(executor.invoke("missing.capability", {}))

    assert outcome.success is None
    assert outcome.error is not None
    assert outcome.error.code == "UNKNOWN_CAPABILITY"
    assert outcome.error.retryable is False


def test_submit_task_runs_in_background_and_stores_result() -> None:
    registry = CapabilityRegistry()

    async def delayed_echo(payload: dict) -> dict:
        await asyncio.sleep(0.01)
        return {"echo": payload["message"]}

    registry.register("system.echo", delayed_echo)
    executor = CapabilityExecutor(registry=registry)

    async def scenario() -> None:
        submitted = await executor.submit_task("system.echo", {"message": "async-hi"})
        assert submitted.error is None
        assert submitted.task is not None
        assert submitted.task.status == "queued"

        status = await _wait_for_task(executor, submitted.task.id)
        assert status is not None
        assert status.status == "succeeded"
        assert status.result == {"echo": "async-hi"}
        assert status.error is None

    asyncio.run(scenario())
