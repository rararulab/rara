-- Kernel audit event log for persistent audit trail

CREATE TABLE IF NOT EXISTS kernel_audit_events (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    timestamp   TIMESTAMPTZ NOT NULL DEFAULT now(),
    agent_id    UUID NOT NULL,
    session_id  TEXT NOT NULL,
    user_id     TEXT NOT NULL,
    event_type  TEXT NOT NULL,
    event_data  JSONB NOT NULL DEFAULT '{}',
    details     JSONB NOT NULL DEFAULT '{}',
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_audit_agent_ts ON kernel_audit_events (agent_id, timestamp);
CREATE INDEX IF NOT EXISTS idx_audit_user_ts ON kernel_audit_events (user_id, timestamp);
CREATE INDEX IF NOT EXISTS idx_audit_event_type ON kernel_audit_events (event_type);
