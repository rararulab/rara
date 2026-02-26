import pytest

from python_worker.capabilities.jobspy import jobspy_scrape


@pytest.mark.integration
def test_jobspy_scrape_live_indeed_remote_returns_records() -> None:
    payload = {
        "site_name": ["indeed"],
        "search_term": "software engineer",
        "location": "Remote",
        "results_wanted": 1,
        "country_indeed": "usa",
    }

    result = jobspy_scrape(payload)

    assert isinstance(result, list)
    assert len(result) >= 1
    first = result[0]
    assert isinstance(first, dict)
    assert first.get("site") == "indeed"
