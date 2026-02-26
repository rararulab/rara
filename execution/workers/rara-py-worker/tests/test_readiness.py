from python_worker.app.state import WorkerState
from python_worker.core.registry import CapabilityRegistry


def test_worker_not_ready_before_mark_initialized() -> None:
    state = WorkerState(registry=CapabilityRegistry())

    ready, detail = state.readiness()

    assert ready is False
    assert detail["ready"] is False
    assert detail["initialized"] is False


def test_worker_ready_after_initialization_and_capability_registration() -> None:
    registry = CapabilityRegistry()
    registry.register("job.discovery.jobspy.scrape", object())
    state = WorkerState(registry=registry)
    state.mark_initialized()

    ready, detail = state.readiness()

    assert ready is True
    assert detail["ready"] is True
    assert detail["capability_count"] == 1


def test_worker_readiness_can_require_grpc_startup() -> None:
    registry = CapabilityRegistry()
    registry.register("system.echo", object())
    state = WorkerState(registry=registry, require_grpc_ready=True)
    state.mark_initialized()

    ready, _ = state.readiness()
    assert ready is False

    state.mark_grpc_ready()
    ready, detail = state.readiness()
    assert ready is True
    assert detail["grpc_ready"] is True
