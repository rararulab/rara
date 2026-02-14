-- Restore notification_log dropped in the up migration.

CREATE TABLE IF NOT EXISTS notification_log (
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

CREATE INDEX IF NOT EXISTS idx_notification_channel ON notification_log (channel);
CREATE INDEX IF NOT EXISTS idx_notification_status ON notification_log (status);
CREATE INDEX IF NOT EXISTS idx_notification_recipient ON notification_log (recipient);
CREATE INDEX IF NOT EXISTS idx_notification_created ON notification_log (created_at);
CREATE INDEX IF NOT EXISTS idx_notification_reference ON notification_log (reference_type, reference_id);
