"""Worker identity model returned by the Status RPC."""

from __future__ import annotations

from dataclasses import dataclass


@dataclass(slots=True, frozen=True)
class WorkerIdentity:
    name: str
    kind: str
