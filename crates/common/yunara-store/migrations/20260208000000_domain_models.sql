-- Domain model migration for job automation platform
-- Entities: job, resume, application, application_status_history,
--           interview_plan, prompt_template, ai_run, notification_log, metrics_snapshot

--------------------------------------------------------------------------------
-- ENUM types
--------------------------------------------------------------------------------

CREATE TYPE job_status AS ENUM ('active', 'archived', 'closed');

CREATE TYPE resume_source AS ENUM ('manual', 'ai_generated', 'optimized');

CREATE TYPE application_channel AS ENUM ('direct', 'referral', 'linkedin', 'email', 'other');

CREATE TYPE application_status AS ENUM (
    'draft',
    'submitted',
    'in_progress',
    'interviewing',
    'offered',
    'rejected',
    'withdrawn',
    'accepted'
);

CREATE TYPE interview_task_status AS ENUM ('pending', 'in_progress', 'completed', 'skipped');

CREATE TYPE prompt_kind AS ENUM (
    'resume_optimize',
    'cover_letter',
    'interview_prep',
    'job_match',
    'follow_up',
    'other'
);

CREATE TYPE ai_model_provider AS ENUM ('openai', 'anthropic', 'local', 'other');

CREATE TYPE notification_channel AS ENUM ('telegram', 'email', 'webhook', 'other');

CREATE TYPE notification_status AS ENUM ('pending', 'sent', 'failed', 'retrying');

CREATE TYPE metrics_period AS ENUM ('daily', 'weekly', 'monthly');

--------------------------------------------------------------------------------
-- job: Job posting info, source, lifecycle
--------------------------------------------------------------------------------

CREATE TABLE job (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    -- Idempotent key: source_job_id + source_name
    source_job_id   TEXT NOT NULL,
    source_name     TEXT NOT NULL,
    title           TEXT NOT NULL,
    company         TEXT NOT NULL,
    location        TEXT,
    description     TEXT,
    url             TEXT,
    salary_min      INTEGER,
    salary_max      INTEGER,
    salary_currency TEXT,
    tags            TEXT[] NOT NULL DEFAULT '{}',
    status          job_status NOT NULL DEFAULT 'active',
    raw_data        JSONB,
    trace_id        TEXT,
    -- Soft delete / archive
    is_deleted      BOOLEAN NOT NULL DEFAULT FALSE,
    deleted_at      TIMESTAMPTZ,
    -- Timestamps
    posted_at       TIMESTAMPTZ,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT uq_job_source UNIQUE (source_job_id, source_name)
);

CREATE INDEX idx_job_title ON job (title);
CREATE INDEX idx_job_company ON job (company);
CREATE INDEX idx_job_location ON job (location);
CREATE INDEX idx_job_status ON job (status);
CREATE INDEX idx_job_updated_at ON job (updated_at);
CREATE INDEX idx_job_source_name ON job (source_name);
CREATE INDEX idx_job_is_deleted ON job (is_deleted) WHERE is_deleted = FALSE;

--------------------------------------------------------------------------------
-- resume: Resume version tracking
--------------------------------------------------------------------------------

CREATE TABLE resume (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    version_tag     TEXT NOT NULL,
    content_hash    TEXT NOT NULL,
    source          resume_source NOT NULL DEFAULT 'manual',
    content         TEXT,
    metadata        JSONB,
    target_job_id   UUID REFERENCES job(id),
    trace_id        TEXT,
    is_deleted      BOOLEAN NOT NULL DEFAULT FALSE,
    deleted_at      TIMESTAMPTZ,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_resume_content_hash ON resume (content_hash);
CREATE INDEX idx_resume_source ON resume (source);
CREATE INDEX idx_resume_target_job ON resume (target_job_id);

--------------------------------------------------------------------------------
-- application: Application record
--------------------------------------------------------------------------------

CREATE TABLE application (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    job_id          UUID NOT NULL REFERENCES job(id),
    resume_id       UUID REFERENCES resume(id),
    channel         application_channel NOT NULL DEFAULT 'direct',
    status          application_status NOT NULL DEFAULT 'draft',
    cover_letter    TEXT,
    notes           TEXT,
    trace_id        TEXT,
    is_deleted      BOOLEAN NOT NULL DEFAULT FALSE,
    deleted_at      TIMESTAMPTZ,
    submitted_at    TIMESTAMPTZ,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_application_job ON application (job_id);
CREATE INDEX idx_application_resume ON application (resume_id);
CREATE INDEX idx_application_status ON application (status);
CREATE INDEX idx_application_submitted_at ON application (submitted_at);

--------------------------------------------------------------------------------
-- application_status_history: Status change trail
--------------------------------------------------------------------------------

CREATE TABLE application_status_history (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    application_id  UUID NOT NULL REFERENCES application(id),
    from_status     application_status,
    to_status       application_status NOT NULL,
    changed_by      TEXT,
    note            TEXT,
    trace_id        TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_app_status_history_app ON application_status_history (application_id);
CREATE INDEX idx_app_status_history_created ON application_status_history (created_at);

--------------------------------------------------------------------------------
-- interview_plan: Interview prep tasks and materials
--------------------------------------------------------------------------------

CREATE TABLE interview_plan (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    application_id  UUID NOT NULL REFERENCES application(id),
    title           TEXT NOT NULL,
    description     TEXT,
    scheduled_at    TIMESTAMPTZ,
    task_status     interview_task_status NOT NULL DEFAULT 'pending',
    materials       JSONB,
    notes           TEXT,
    trace_id        TEXT,
    is_deleted      BOOLEAN NOT NULL DEFAULT FALSE,
    deleted_at      TIMESTAMPTZ,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_interview_plan_app ON interview_plan (application_id);
CREATE INDEX idx_interview_plan_status ON interview_plan (task_status);
CREATE INDEX idx_interview_plan_scheduled ON interview_plan (scheduled_at);

--------------------------------------------------------------------------------
-- prompt_template: AI template library by kind/version
--------------------------------------------------------------------------------

CREATE TABLE prompt_template (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name            TEXT NOT NULL,
    kind            prompt_kind NOT NULL,
    version         INTEGER NOT NULL DEFAULT 1,
    content         TEXT NOT NULL,
    description     TEXT,
    is_active       BOOLEAN NOT NULL DEFAULT TRUE,
    metadata        JSONB,
    trace_id        TEXT,
    is_deleted      BOOLEAN NOT NULL DEFAULT FALSE,
    deleted_at      TIMESTAMPTZ,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT uq_prompt_template_kind_version UNIQUE (kind, name, version)
);

CREATE INDEX idx_prompt_template_kind ON prompt_template (kind);
CREATE INDEX idx_prompt_template_active ON prompt_template (is_active) WHERE is_active = TRUE;

--------------------------------------------------------------------------------
-- ai_run: Model call records, I/O summary, token/cost
--------------------------------------------------------------------------------

CREATE TABLE ai_run (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    template_id     UUID REFERENCES prompt_template(id),
    model_name      TEXT NOT NULL,
    provider        ai_model_provider NOT NULL,
    input_summary   TEXT,
    output_summary  TEXT,
    input_tokens    INTEGER NOT NULL DEFAULT 0,
    output_tokens   INTEGER NOT NULL DEFAULT 0,
    total_tokens    INTEGER NOT NULL DEFAULT 0,
    cost_cents      INTEGER NOT NULL DEFAULT 0,
    duration_ms     INTEGER NOT NULL DEFAULT 0,
    is_success      BOOLEAN NOT NULL DEFAULT TRUE,
    error_message   TEXT,
    metadata        JSONB,
    trace_id        TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_ai_run_template ON ai_run (template_id);
CREATE INDEX idx_ai_run_model ON ai_run (model_name);
CREATE INDEX idx_ai_run_provider ON ai_run (provider);
CREATE INDEX idx_ai_run_created ON ai_run (created_at);

--------------------------------------------------------------------------------
-- notification_log: Telegram/Email notification records
--------------------------------------------------------------------------------

CREATE TABLE notification_log (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    channel         notification_channel NOT NULL,
    recipient       TEXT NOT NULL,
    subject         TEXT,
    body            TEXT NOT NULL,
    status          notification_status NOT NULL DEFAULT 'pending',
    retry_count     INTEGER NOT NULL DEFAULT 0,
    error_message   TEXT,
    reference_type  TEXT,
    reference_id    UUID,
    metadata        JSONB,
    trace_id        TEXT,
    sent_at         TIMESTAMPTZ,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_notification_channel ON notification_log (channel);
CREATE INDEX idx_notification_status ON notification_log (status);
CREATE INDEX idx_notification_recipient ON notification_log (recipient);
CREATE INDEX idx_notification_created ON notification_log (created_at);
CREATE INDEX idx_notification_reference ON notification_log (reference_type, reference_id);

--------------------------------------------------------------------------------
-- metrics_snapshot: Stats snapshot (daily/weekly)
--------------------------------------------------------------------------------

CREATE TABLE metrics_snapshot (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    period          metrics_period NOT NULL,
    snapshot_date   DATE NOT NULL,
    jobs_discovered INTEGER NOT NULL DEFAULT 0,
    applications_sent INTEGER NOT NULL DEFAULT 0,
    interviews_scheduled INTEGER NOT NULL DEFAULT 0,
    offers_received INTEGER NOT NULL DEFAULT 0,
    rejections      INTEGER NOT NULL DEFAULT 0,
    ai_runs_count   INTEGER NOT NULL DEFAULT 0,
    ai_total_cost_cents INTEGER NOT NULL DEFAULT 0,
    extra           JSONB,
    trace_id        TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT uq_metrics_snapshot_period_date UNIQUE (period, snapshot_date)
);

CREATE INDEX idx_metrics_period ON metrics_snapshot (period);
CREATE INDEX idx_metrics_date ON metrics_snapshot (snapshot_date);

--------------------------------------------------------------------------------
-- Trigger function for auto-updating updated_at
--------------------------------------------------------------------------------

CREATE OR REPLACE FUNCTION trigger_set_updated_at()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = now();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

-- Apply updated_at triggers to tables with updated_at column
CREATE TRIGGER set_updated_at BEFORE UPDATE ON job
    FOR EACH ROW EXECUTE FUNCTION trigger_set_updated_at();

CREATE TRIGGER set_updated_at BEFORE UPDATE ON resume
    FOR EACH ROW EXECUTE FUNCTION trigger_set_updated_at();

CREATE TRIGGER set_updated_at BEFORE UPDATE ON application
    FOR EACH ROW EXECUTE FUNCTION trigger_set_updated_at();

CREATE TRIGGER set_updated_at BEFORE UPDATE ON interview_plan
    FOR EACH ROW EXECUTE FUNCTION trigger_set_updated_at();

CREATE TRIGGER set_updated_at BEFORE UPDATE ON prompt_template
    FOR EACH ROW EXECUTE FUNCTION trigger_set_updated_at();
