-- Restore legacy scheduler tables (copied verbatim from 20260304000000_init.up.sql).

CREATE TABLE scheduler_task (
    id            TEXT NOT NULL PRIMARY KEY,
    name          TEXT NOT NULL UNIQUE,
    cron_expr     TEXT NOT NULL,
    enabled       INTEGER NOT NULL DEFAULT 1,
    last_run_at   TEXT,
    last_status   INTEGER,
    last_error    TEXT,
    run_count     INTEGER NOT NULL DEFAULT 0,
    failure_count INTEGER NOT NULL DEFAULT 0,
    is_deleted    INTEGER NOT NULL DEFAULT 0,
    deleted_at    TEXT,
    created_at    TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at    TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX idx_scheduler_task_name ON scheduler_task(name);
CREATE INDEX idx_scheduler_task_enabled ON scheduler_task(enabled)
    WHERE is_deleted = 0;

CREATE TRIGGER set_scheduler_task_updated_at AFTER UPDATE ON scheduler_task
BEGIN
    UPDATE scheduler_task SET updated_at = datetime('now') WHERE id = NEW.id;
END;

CREATE TABLE task_run_history (
    id          TEXT NOT NULL PRIMARY KEY,
    task_id     TEXT NOT NULL REFERENCES scheduler_task(id),
    status      INTEGER NOT NULL,
    started_at  TEXT NOT NULL,
    finished_at TEXT,
    duration_ms INTEGER,
    error       TEXT,
    output      TEXT,
    created_at  TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX idx_task_run_history_task_id ON task_run_history(task_id);
CREATE INDEX idx_task_run_history_started_at ON task_run_history(started_at DESC);
