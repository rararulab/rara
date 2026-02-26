"""Thin mem0 SDK capability handlers."""

from __future__ import annotations

from importlib import import_module
from typing import Any

MEMORY_INSTANCE: Any | None = None


def _require_memory_instance() -> Any:
    if MEMORY_INSTANCE is None:
        raise RuntimeError("mem0 instance is not configured; call mem0.from_config first")
    return MEMORY_INSTANCE


def mem0_from_config(payload: dict[str, Any]) -> None:
    """Initialize/replace the module-level mem0 SDK instance."""
    global MEMORY_INSTANCE
    mem0 = import_module("mem0")
    MEMORY_INSTANCE = mem0.Memory.from_config(payload)
    return None


def mem0_add(payload: dict[str, Any]) -> Any:
    return _require_memory_instance().add(**payload)


def mem0_get_all(payload: dict[str, Any]) -> Any:
    return _require_memory_instance().get_all(**payload)


def mem0_get(payload: dict[str, Any]) -> Any:
    return _require_memory_instance().get(**payload)


def mem0_search(payload: dict[str, Any]) -> Any:
    return _require_memory_instance().search(**payload)


def mem0_update(payload: dict[str, Any]) -> Any:
    return _require_memory_instance().update(**payload)


def mem0_history(payload: dict[str, Any]) -> Any:
    return _require_memory_instance().history(**payload)


def mem0_delete(payload: dict[str, Any]) -> Any:
    return _require_memory_instance().delete(**payload)


def mem0_delete_all(payload: dict[str, Any]) -> Any:
    return _require_memory_instance().delete_all(**payload)


def mem0_reset(payload: dict[str, Any]) -> Any:
    return _require_memory_instance().reset(**payload)
