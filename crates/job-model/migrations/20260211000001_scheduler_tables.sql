-- Scheduler task metadata and run history tables
CREATE TYPE task_run_status AS ENUM ('success', 'failed', 'running');

CREATE TABLE scheduler_task (
    id UUID PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    cron_expr TEXT NOT NULL,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    last_run_at TIMESTAMPTZ,
    last_status task_run_status,
    last_error TEXT,
    run_count BIGINT NOT NULL DEFAULT 0,
    failure_count BIGINT NOT NULL DEFAULT 0,
    is_deleted BOOLEAN NOT NULL DEFAULT FALSE,
    deleted_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE task_run_history (
    id UUID PRIMARY KEY,
    task_id UUID NOT NULL REFERENCES scheduler_task(id),
    status task_run_status NOT NULL,
    started_at TIMESTAMPTZ NOT NULL,
    finished_at TIMESTAMPTZ,
    duration_ms BIGINT,
    error TEXT,
    output JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_scheduler_task_name ON scheduler_task(name);
CREATE INDEX idx_scheduler_task_enabled ON scheduler_task(enabled) WHERE is_deleted = FALSE;
CREATE INDEX idx_task_run_history_task_id ON task_run_history(task_id);
CREATE INDEX idx_task_run_history_started_at ON task_run_history(started_at DESC);

CREATE TRIGGER set_scheduler_task_updated_at BEFORE UPDATE ON scheduler_task
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();
