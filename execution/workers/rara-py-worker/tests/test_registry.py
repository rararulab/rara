from python_worker.core.registry import CapabilityRegistry


def test_registry_tracks_registered_capabilities() -> None:
    registry = CapabilityRegistry()
    registry.register("job.discovery.jobspy.scrape", object())

    assert registry.has("job.discovery.jobspy.scrape") is True
    assert registry.has("missing.capability") is False
    assert registry.names() == ["job.discovery.jobspy.scrape"]

