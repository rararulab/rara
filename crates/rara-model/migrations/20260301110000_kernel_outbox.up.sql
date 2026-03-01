CREATE TABLE IF NOT EXISTS kernel_outbox (
    id           TEXT PRIMARY KEY,
    channel_type TEXT NOT NULL,
    target       JSONB NOT NULL,
    payload      JSONB NOT NULL,
    status       SMALLINT NOT NULL DEFAULT 0,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    delivered_at TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS idx_outbox_pending
    ON kernel_outbox (status, created_at) WHERE status = 0;
