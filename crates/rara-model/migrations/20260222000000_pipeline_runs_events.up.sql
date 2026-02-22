CREATE TABLE pipeline_runs (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    status      SMALLINT NOT NULL DEFAULT 0,
    started_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    finished_at TIMESTAMPTZ,
    jobs_found  INT NOT NULL DEFAULT 0,
    jobs_scored INT NOT NULL DEFAULT 0,
    jobs_applied INT NOT NULL DEFAULT 0,
    jobs_notified INT NOT NULL DEFAULT 0,
    summary     TEXT,
    error       TEXT
);

CREATE TABLE pipeline_events (
    id          BIGSERIAL PRIMARY KEY,
    run_id      UUID NOT NULL REFERENCES pipeline_runs(id) ON DELETE CASCADE,
    seq         INT NOT NULL,
    event_type  VARCHAR(64) NOT NULL,
    payload     JSONB NOT NULL DEFAULT '{}',
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_pipeline_events_run_seq ON pipeline_events(run_id, seq);
