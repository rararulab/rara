-- Revert: restore the original pipeline_discovered_jobs with duplicated fields.

DROP TABLE IF EXISTS pipeline_discovered_jobs;

CREATE TABLE pipeline_discovered_jobs (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    run_id      UUID NOT NULL REFERENCES pipeline_runs(id) ON DELETE CASCADE,
    title       TEXT NOT NULL,
    company     TEXT,
    location    TEXT,
    url         TEXT,
    description TEXT,
    score       INT,
    action      SMALLINT NOT NULL DEFAULT 0,
    date_posted TEXT,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_pdj_run_id ON pipeline_discovered_jobs(run_id);
