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
