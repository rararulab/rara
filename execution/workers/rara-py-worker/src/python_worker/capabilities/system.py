from __future__ import annotations

from typing import Any


async def health_ping(payload: dict[str, Any]) -> dict[str, Any]:
    return {"ok": True, "echo": payload}


def system_echo(payload: dict[str, Any]) -> dict[str, Any]:
    return {"echo": payload}

