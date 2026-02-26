from python_worker.app.container import build_container


def test_container_uses_env_worker_name_and_python_kind(monkeypatch) -> None:
    monkeypatch.setenv("RARA_WORKER_NAME", "search-job-42")

    container = build_container()

    assert container.identity.name == "search-job-42"
    assert container.identity.kind == "python"


def test_container_defaults_worker_name(monkeypatch) -> None:
    monkeypatch.delenv("RARA_WORKER_NAME", raising=False)

    container = build_container()

    assert container.identity.name == "rara-py-worker"
    assert container.identity.kind == "python"


def test_container_registers_jobspy_capability() -> None:
    container = build_container()

    assert container.state.registry.has("jobspy.scrape_jobs") is True


def test_container_registers_mem0_capabilities() -> None:
    container = build_container()

    for capability in (
        "mem0.from_config",
        "mem0.add",
        "mem0.get_all",
        "mem0.get",
        "mem0.search",
        "mem0.update",
        "mem0.history",
        "mem0.delete",
        "mem0.delete_all",
        "mem0.reset",
    ):
        assert container.state.registry.has(capability) is True
