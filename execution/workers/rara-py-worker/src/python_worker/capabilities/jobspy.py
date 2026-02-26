"""jobspy capability handlers."""

from __future__ import annotations

from importlib import import_module
from typing import Any


def _coerce_jobspy_result(value: Any) -> Any:
    """Convert common jobspy return values into JSON-serializable structures."""
    if isinstance(value, (dict, list, str, int, float, bool)) or value is None:
        return value

    to_dict = getattr(value, "to_dict", None)
    if callable(to_dict):
        try:
            return to_dict("records")
        except TypeError:
            return to_dict(orient="records")

    return value


def jobspy_scrape(payload: dict[str, Any]) -> Any:
    """Pass payload directly to ``jobspy.scrape_jobs`` and return raw results."""
    jobspy = import_module("jobspy")
    result = jobspy.scrape_jobs(**payload)
    return _coerce_jobspy_result(result)
