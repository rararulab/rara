-- Saved job tracking: URL-based job saving with crawl/analysis pipeline.
-- status SMALLINT: pending_crawl=0, crawling=1, crawled=2, analyzing=3,
--                  analyzed=4, failed=5, expired=6

CREATE TABLE saved_job (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    url TEXT NOT NULL UNIQUE,
    title TEXT,
    company TEXT,
    status SMALLINT NOT NULL DEFAULT 0,
    markdown_s3_key TEXT,
    markdown_preview TEXT,
    analysis_result JSONB,
    match_score REAL,
    error_message TEXT,
    crawled_at TIMESTAMPTZ,
    analyzed_at TIMESTAMPTZ,
    expires_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TRIGGER saved_job_updated_at BEFORE UPDATE ON saved_job
    FOR EACH ROW EXECUTE FUNCTION trigger_set_updated_at();

CREATE INDEX idx_saved_job_status ON saved_job(status);
CREATE INDEX idx_saved_job_url ON saved_job(url);
