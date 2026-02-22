-- Re-create saved_job tables (rollback)
CREATE TABLE IF NOT EXISTS saved_job (
    id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    url              TEXT NOT NULL UNIQUE,
    title            TEXT,
    company          TEXT,
    status           SMALLINT NOT NULL DEFAULT 0,
    markdown_s3_key  TEXT,
    markdown_preview TEXT,
    analysis_result  JSONB,
    match_score      REAL,
    error_message    TEXT,
    crawled_at       TIMESTAMPTZ,
    analyzed_at      TIMESTAMPTZ,
    expires_at       TIMESTAMPTZ,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS saved_job_event (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    saved_job_id  UUID NOT NULL REFERENCES saved_job(id) ON DELETE CASCADE,
    stage         SMALLINT NOT NULL,
    event_kind    SMALLINT NOT NULL,
    message       TEXT NOT NULL DEFAULT '',
    metadata      JSONB,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_saved_job_event_saved_job_id ON saved_job_event(saved_job_id);
