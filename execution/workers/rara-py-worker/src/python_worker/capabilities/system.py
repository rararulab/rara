"""Small built-in capabilities used for local testing and smoke checks."""

from __future__ import annotations

from typing import Any


async def health_ping(payload: dict[str, Any]) -> dict[str, Any]:
    """Async health capability that echoes payload for connectivity checks."""
    return {"ok": True, "echo": payload}


def system_echo(payload: dict[str, Any]) -> dict[str, Any]:
    """Simple sync echo capability used in unit tests."""
    return {"echo": payload}
