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
    global _INIT_DONE
    if _INIT_DONE:
        return

    service_name = os.getenv(_SERVICE_NAME_ENV, _DEFAULT_SERVICE_NAME)
    provider = TracerProvider(resource=Resource.create({"service.name": service_name}))
    provider.add_span_processor(SimpleSpanProcessor(ConsoleSpanExporter()))
    trace.set_tracer_provider(provider)
    _INIT_DONE = True


def get_tracer(name: str):
    return trace.get_tracer(name)
