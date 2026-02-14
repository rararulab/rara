-- Restore prompt_template and ai_run dropped in the up migration.

DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_type WHERE typname = 'prompt_kind') THEN
        CREATE TYPE prompt_kind AS ENUM (
            'resume_optimize', 'cover_letter', 'interview_prep',
            'job_match', 'follow_up', 'other'
        );
    END IF;
END;
$$;

DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_type WHERE typname = 'ai_model_provider') THEN
        CREATE TYPE ai_model_provider AS ENUM ('openai', 'anthropic', 'local', 'other');
    END IF;
END;
$$;

CREATE TABLE IF NOT EXISTS prompt_template (
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

CREATE INDEX IF NOT EXISTS idx_prompt_template_kind ON prompt_template (kind);
CREATE INDEX IF NOT EXISTS idx_prompt_template_active ON prompt_template (is_active) WHERE is_active = TRUE;

DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_proc WHERE proname = 'trigger_set_updated_at') THEN
        IF NOT EXISTS (
            SELECT 1
            FROM pg_trigger
            WHERE tgname = 'set_updated_at'
              AND tgrelid = 'prompt_template'::regclass
        ) THEN
            CREATE TRIGGER set_updated_at
                BEFORE UPDATE ON prompt_template
                FOR EACH ROW EXECUTE FUNCTION trigger_set_updated_at();
        END IF;
    END IF;
END;
$$;

CREATE TABLE IF NOT EXISTS ai_run (
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

CREATE INDEX IF NOT EXISTS idx_ai_run_template ON ai_run (template_id);
CREATE INDEX IF NOT EXISTS idx_ai_run_model ON ai_run (model_name);
CREATE INDEX IF NOT EXISTS idx_ai_run_provider ON ai_run (provider);
CREATE INDEX IF NOT EXISTS idx_ai_run_created ON ai_run (created_at);
