from __future__ import annotations


class CapabilityRegistry:
    """Stores capability handlers by name."""

    def __init__(self) -> None:
        self._handlers: dict[str, object] = {}

    def register(self, capability: str, handler: object) -> None:
        if not capability:
            raise ValueError("capability must not be empty")
        self._handlers[capability] = handler

    def has(self, capability: str) -> bool:
        return capability in self._handlers

    def get(self, capability: str) -> object | None:
        return self._handlers.get(capability)

    def names(self) -> list[str]:
        return sorted(self._handlers.keys())

    def count(self) -> int:
        return len(self._handlers)
