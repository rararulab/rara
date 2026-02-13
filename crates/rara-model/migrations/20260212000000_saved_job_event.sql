CREATE TABLE saved_job_event (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    saved_job_id UUID NOT NULL REFERENCES saved_job(id) ON DELETE CASCADE,
    stage SMALLINT NOT NULL,
    event_kind SMALLINT NOT NULL,
    message TEXT NOT NULL,
    metadata JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_saved_job_event_job_id ON saved_job_event(saved_job_id);
CREATE INDEX idx_saved_job_event_timeline ON saved_job_event(saved_job_id, created_at);
