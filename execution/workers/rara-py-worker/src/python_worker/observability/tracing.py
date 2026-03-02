# Copyright 2025 Rararulab
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

"""OpenTelemetry tracing setup with stdout exporter for local/K8s logs."""

from __future__ import annotations

import os
from typing import Final

from opentelemetry import trace
from opentelemetry.sdk.resources import Resource
from opentelemetry.sdk.trace import TracerProvider
from opentelemetry.sdk.trace.export import ConsoleSpanExporter, SimpleSpanProcessor

_SERVICE_NAME_ENV: Final[str] = "OTEL_SERVICE_NAME"
_DEFAULT_SERVICE_NAME: Final[str] = "rara-py-worker"
_INIT_DONE = False


def init_tracing() -> None:
    """Initialize a global tracer provider once using ConsoleSpanExporter."""
    global _INIT_DONE
    if _INIT_DONE:
        return

    service_name = os.getenv(_SERVICE_NAME_ENV, _DEFAULT_SERVICE_NAME)
    provider = TracerProvider(resource=Resource.create({"service.name": service_name}))
    provider.add_span_processor(SimpleSpanProcessor(ConsoleSpanExporter()))
    trace.set_tracer_provider(provider)
    _INIT_DONE = True


def get_tracer(name: str):
    """Return a tracer from the global provider."""
    return trace.get_tracer(name)
