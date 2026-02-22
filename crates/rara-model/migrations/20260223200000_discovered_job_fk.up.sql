-- Replace duplicated job fields in pipeline_discovered_jobs with a job_id FK.
-- Since this is dev, we DROP and re-CREATE the table.

DROP TABLE IF EXISTS pipeline_discovered_jobs;

CREATE TABLE pipeline_discovered_jobs (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    run_id      UUID NOT NULL REFERENCES pipeline_runs(id) ON DELETE CASCADE,
    job_id      UUID NOT NULL REFERENCES job(id) ON DELETE CASCADE,
    score       INT,
    action      SMALLINT NOT NULL DEFAULT 0,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_pdj_run_id ON pipeline_discovered_jobs(run_id);
CREATE INDEX idx_pdj_job_id ON pipeline_discovered_jobs(job_id);
