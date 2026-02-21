CREATE TABLE IF NOT EXISTS coding_task (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    status          SMALLINT NOT NULL DEFAULT 0,
    agent_type      SMALLINT NOT NULL DEFAULT 0,
    repo_url        TEXT NOT NULL,
    branch          TEXT NOT NULL,
    prompt          TEXT NOT NULL,
    pr_url          TEXT,
    pr_number       INTEGER,
    session_key     TEXT,
    tmux_session    TEXT NOT NULL DEFAULT '',
    workspace_path  TEXT NOT NULL DEFAULT '',
    output          TEXT NOT NULL DEFAULT '',
    exit_code       INTEGER,
    error           TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    started_at      TIMESTAMPTZ,
    completed_at    TIMESTAMPTZ
);

CREATE INDEX idx_coding_task_status ON coding_task(status);
CREATE INDEX idx_coding_task_created ON coding_task(created_at DESC);
