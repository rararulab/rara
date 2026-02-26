from __future__ import annotations

from types import SimpleNamespace

import pytest

from python_worker.capabilities import mem0 as mem0_capability


@pytest.fixture(autouse=True)
def reset_mem0_instance() -> None:
    mem0_capability.MEMORY_INSTANCE = None


def test_mem0_from_config_sets_global_instance(monkeypatch) -> None:
    created = object()

    class FakeMemory:
        @staticmethod
        def from_config(config: dict):
            assert config == {"version": "v1.1"}
            return created

    fake_module = SimpleNamespace(Memory=FakeMemory)
    monkeypatch.setattr(mem0_capability, "import_module", lambda _: fake_module)

    result = mem0_capability.mem0_from_config({"version": "v1.1"})

    assert result is None
    assert mem0_capability.MEMORY_INSTANCE is created


def test_mem0_search_forwards_payload_to_sdk_instance() -> None:
    calls: list[tuple[str, dict]] = []

    class FakeMemoryInstance:
        def search(self, **kwargs):
            calls.append(("search", kwargs))
            return [{"id": "m1", "memory": "rust"}]

    mem0_capability.MEMORY_INSTANCE = FakeMemoryInstance()

    result = mem0_capability.mem0_search({"query": "rust", "user_id": "u1"})

    assert calls == [("search", {"query": "rust", "user_id": "u1"})]
    assert result == [{"id": "m1", "memory": "rust"}]


def test_mem0_methods_require_configuration() -> None:
    with pytest.raises(RuntimeError, match="mem0 instance is not configured"):
        mem0_capability.mem0_get({"memory_id": "x"})
