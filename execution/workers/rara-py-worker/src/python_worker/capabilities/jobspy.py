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
