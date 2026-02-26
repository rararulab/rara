from python_worker.core.registry import CapabilityRegistry


def test_registry_tracks_registered_capabilities() -> None:
    registry = CapabilityRegistry()
    registry.register("jobspy.scrape_jobs", object())

    assert registry.has("jobspy.scrape_jobs") is True
    assert registry.has("missing.capability") is False
    assert registry.names() == ["jobspy.scrape_jobs"]
