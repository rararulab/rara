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
