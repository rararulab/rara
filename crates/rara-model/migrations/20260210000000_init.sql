-- Consolidated init migration for job automation platform.
-- All enum columns use SMALLINT codes (aligned with Rust #[repr(u8)]).

-- Extensions
CREATE EXTENSION IF NOT EXISTS "pgcrypto";

--------------------------------------------------------------------------------
-- PG ENUM types (only for columns not yet migrated to SMALLINT)
--------------------------------------------------------------------------------

CREATE TYPE job_status AS ENUM ('active', 'archived', 'closed');

CREATE TYPE prompt_kind AS ENUM (
    'resume_optimize', 'cover_letter', 'interview_prep',
    'job_match', 'follow_up', 'other'
);

CREATE TYPE ai_model_provider AS ENUM ('openai', 'anthropic', 'local', 'other');

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

--------------------------------------------------------------------------------
-- kv_table: Key-value storage
--------------------------------------------------------------------------------

CREATE TABLE kv_table (
    key   TEXT NOT NULL PRIMARY KEY,
    value TEXT
);

--------------------------------------------------------------------------------
-- job: Job posting info, source, lifecycle
--------------------------------------------------------------------------------

CREATE TABLE job (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
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
    is_deleted      BOOLEAN NOT NULL DEFAULT FALSE,
    deleted_at      TIMESTAMPTZ,
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

CREATE TRIGGER set_updated_at BEFORE UPDATE ON job
    FOR EACH ROW EXECUTE FUNCTION trigger_set_updated_at();

--------------------------------------------------------------------------------
-- resume: Resume version tracking
-- source SMALLINT: manual=0, ai_generated=1, optimized=2
--------------------------------------------------------------------------------

CREATE TABLE resume (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    title               TEXT NOT NULL DEFAULT '',
    version_tag         TEXT NOT NULL,
    content_hash        TEXT NOT NULL,
    source              SMALLINT NOT NULL DEFAULT 0,
    content             TEXT,
    metadata            JSONB,
    target_job_id       UUID REFERENCES job(id),
    parent_resume_id    UUID REFERENCES resume(id),
    customization_notes TEXT,
    tags                TEXT[] NOT NULL DEFAULT '{}',
    trace_id            TEXT,
    is_deleted          BOOLEAN NOT NULL DEFAULT FALSE,
    deleted_at          TIMESTAMPTZ,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_resume_content_hash ON resume (content_hash);
CREATE INDEX idx_resume_source ON resume (source);
CREATE INDEX idx_resume_target_job ON resume (target_job_id);
CREATE INDEX idx_resume_parent ON resume (parent_resume_id);
CREATE INDEX idx_resume_is_deleted ON resume (is_deleted) WHERE is_deleted = FALSE;

CREATE TRIGGER set_updated_at BEFORE UPDATE ON resume
    FOR EACH ROW EXECUTE FUNCTION trigger_set_updated_at();

--------------------------------------------------------------------------------
-- application: Application record
-- channel  SMALLINT: direct=0, referral=1, linkedin=2, email=3, other=4
-- status   SMALLINT: draft=0, submitted=1, under_review=2, interview=3,
--                    offered=4, rejected=5, accepted=6, withdrawn=7
-- priority SMALLINT: low=0, medium=1, high=2, critical=3
--------------------------------------------------------------------------------

CREATE TABLE application (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    job_id       UUID NOT NULL REFERENCES job(id),
    resume_id    UUID REFERENCES resume(id),
    channel      SMALLINT NOT NULL DEFAULT 0,
    status       SMALLINT NOT NULL DEFAULT 0,
    priority     SMALLINT NOT NULL DEFAULT 1,
    cover_letter TEXT,
    notes        TEXT,
    tags         TEXT[] NOT NULL DEFAULT '{}',
    trace_id     TEXT,
    is_deleted   BOOLEAN NOT NULL DEFAULT FALSE,
    deleted_at   TIMESTAMPTZ,
    submitted_at TIMESTAMPTZ,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_application_job ON application (job_id);
CREATE INDEX idx_application_resume ON application (resume_id);
CREATE INDEX idx_application_status ON application (status);
CREATE INDEX idx_application_submitted_at ON application (submitted_at);

CREATE TRIGGER set_updated_at BEFORE UPDATE ON application
    FOR EACH ROW EXECUTE FUNCTION trigger_set_updated_at();

--------------------------------------------------------------------------------
-- application_status_history: Status change trail
-- from_status / to_status SMALLINT: same codes as application.status
--------------------------------------------------------------------------------

CREATE TABLE application_status_history (
    id             UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    application_id UUID NOT NULL REFERENCES application(id),
    from_status    SMALLINT,
    to_status      SMALLINT NOT NULL,
    changed_by     TEXT,
    note           TEXT,
    trace_id       TEXT,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_app_status_history_app ON application_status_history (application_id);
CREATE INDEX idx_app_status_history_created ON application_status_history (created_at);

--------------------------------------------------------------------------------
-- interview_plan: Interview prep tasks and materials
-- task_status SMALLINT: pending=0, in_progress=1, completed=2, skipped=3
--------------------------------------------------------------------------------

CREATE TABLE interview_plan (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    application_id  UUID NOT NULL REFERENCES application(id),
    title           TEXT NOT NULL,
    description     TEXT,
    company         TEXT NOT NULL DEFAULT '',
    position        TEXT NOT NULL DEFAULT '',
    job_description TEXT,
    round           TEXT NOT NULL DEFAULT 'technical',
    scheduled_at    TIMESTAMPTZ,
    task_status     SMALLINT NOT NULL DEFAULT 0,
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

CREATE TRIGGER set_updated_at BEFORE UPDATE ON interview_plan
    FOR EACH ROW EXECUTE FUNCTION trigger_set_updated_at();

--------------------------------------------------------------------------------
-- prompt_template: AI template library by kind/version
--------------------------------------------------------------------------------

CREATE TABLE prompt_template (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name        TEXT NOT NULL,
    kind        prompt_kind NOT NULL,
    version     INTEGER NOT NULL DEFAULT 1,
    content     TEXT NOT NULL,
    description TEXT,
    is_active   BOOLEAN NOT NULL DEFAULT TRUE,
    metadata    JSONB,
    trace_id    TEXT,
    is_deleted  BOOLEAN NOT NULL DEFAULT FALSE,
    deleted_at  TIMESTAMPTZ,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT uq_prompt_template_kind_version UNIQUE (kind, name, version)
);

CREATE INDEX idx_prompt_template_kind ON prompt_template (kind);
CREATE INDEX idx_prompt_template_active ON prompt_template (is_active) WHERE is_active = TRUE;

CREATE TRIGGER set_updated_at BEFORE UPDATE ON prompt_template
    FOR EACH ROW EXECUTE FUNCTION trigger_set_updated_at();

--------------------------------------------------------------------------------
-- ai_run: Model call records, I/O summary, token/cost
--------------------------------------------------------------------------------

CREATE TABLE ai_run (
    id             UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    template_id    UUID REFERENCES prompt_template(id),
    model_name     TEXT NOT NULL,
    provider       ai_model_provider NOT NULL,
    input_summary  TEXT,
    output_summary TEXT,
    input_tokens   INTEGER NOT NULL DEFAULT 0,
    output_tokens  INTEGER NOT NULL DEFAULT 0,
    total_tokens   INTEGER NOT NULL DEFAULT 0,
    cost_cents     INTEGER NOT NULL DEFAULT 0,
    duration_ms    INTEGER NOT NULL DEFAULT 0,
    is_success     BOOLEAN NOT NULL DEFAULT TRUE,
    error_message  TEXT,
    metadata       JSONB,
    trace_id       TEXT,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_ai_run_template ON ai_run (template_id);
CREATE INDEX idx_ai_run_model ON ai_run (model_name);
CREATE INDEX idx_ai_run_provider ON ai_run (provider);
CREATE INDEX idx_ai_run_created ON ai_run (created_at);

--------------------------------------------------------------------------------
-- notification_log: Telegram/Email notification records
-- channel  SMALLINT: telegram=0, email=1, webhook=2
-- status   SMALLINT: pending=0, sent=1, failed=2, retrying=3
-- priority SMALLINT: low=0, normal=1, high=2, urgent=3
--------------------------------------------------------------------------------

CREATE TABLE notification_log (
    id             UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    channel        SMALLINT NOT NULL,
    recipient      TEXT NOT NULL,
    subject        TEXT,
    body           TEXT NOT NULL,
    status         SMALLINT NOT NULL DEFAULT 0,
    priority       SMALLINT NOT NULL DEFAULT 1,
    retry_count    INTEGER NOT NULL DEFAULT 0,
    max_retries    INTEGER NOT NULL DEFAULT 3,
    error_message  TEXT,
    reference_type TEXT,
    reference_id   UUID,
    metadata       JSONB,
    trace_id       TEXT,
    sent_at        TIMESTAMPTZ,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_notification_channel ON notification_log (channel);
CREATE INDEX idx_notification_status ON notification_log (status);
CREATE INDEX idx_notification_recipient ON notification_log (recipient);
CREATE INDEX idx_notification_created ON notification_log (created_at);
CREATE INDEX idx_notification_reference ON notification_log (reference_type, reference_id);

--------------------------------------------------------------------------------
-- metrics_snapshot: Stats snapshot (daily/weekly/monthly)
-- period SMALLINT: daily=0, weekly=1, monthly=2
--------------------------------------------------------------------------------

CREATE TABLE metrics_snapshot (
    id                    UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    period                SMALLINT NOT NULL,
    snapshot_date         DATE NOT NULL,
    jobs_discovered       INTEGER NOT NULL DEFAULT 0,
    applications_sent     INTEGER NOT NULL DEFAULT 0,
    interviews_scheduled  INTEGER NOT NULL DEFAULT 0,
    offers_received       INTEGER NOT NULL DEFAULT 0,
    rejections            INTEGER NOT NULL DEFAULT 0,
    ai_runs_count         INTEGER NOT NULL DEFAULT 0,
    ai_total_cost_cents   INTEGER NOT NULL DEFAULT 0,
    extra                 JSONB,
    trace_id              TEXT,
    created_at            TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT uq_metrics_snapshot_period_date UNIQUE (period, snapshot_date)
);

CREATE INDEX idx_metrics_period ON metrics_snapshot (period);
CREATE INDEX idx_metrics_date ON metrics_snapshot (snapshot_date);

--------------------------------------------------------------------------------
-- scheduler_task: Cron task metadata
-- last_status SMALLINT: success=0, failed=1, running=2
--------------------------------------------------------------------------------

CREATE TABLE scheduler_task (
    id            UUID PRIMARY KEY,
    name          TEXT NOT NULL UNIQUE,
    cron_expr     TEXT NOT NULL,
    enabled       BOOLEAN NOT NULL DEFAULT TRUE,
    last_run_at   TIMESTAMPTZ,
    last_status   SMALLINT,
    last_error    TEXT,
    run_count     BIGINT NOT NULL DEFAULT 0,
    failure_count BIGINT NOT NULL DEFAULT 0,
    is_deleted    BOOLEAN NOT NULL DEFAULT FALSE,
    deleted_at    TIMESTAMPTZ,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_scheduler_task_name ON scheduler_task(name);
CREATE INDEX idx_scheduler_task_enabled ON scheduler_task(enabled) WHERE is_deleted = FALSE;

CREATE TRIGGER set_scheduler_task_updated_at BEFORE UPDATE ON scheduler_task
    FOR EACH ROW EXECUTE FUNCTION trigger_set_updated_at();

--------------------------------------------------------------------------------
-- task_run_history: Scheduler execution log
-- status SMALLINT: success=0, failed=1, running=2
--------------------------------------------------------------------------------

CREATE TABLE task_run_history (
    id          UUID PRIMARY KEY,
    task_id     UUID NOT NULL REFERENCES scheduler_task(id),
    status      SMALLINT NOT NULL,
    started_at  TIMESTAMPTZ NOT NULL,
    finished_at TIMESTAMPTZ,
    duration_ms BIGINT,
    error       TEXT,
    output      JSONB,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_task_run_history_task_id ON task_run_history(task_id);
CREATE INDEX idx_task_run_history_started_at ON task_run_history(started_at DESC);
