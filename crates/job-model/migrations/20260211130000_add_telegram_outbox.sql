-- telegram_outbox: bot-owned transport outbox for telegram delivery attempts
-- status: pending=0, sent=1, failed=2

CREATE TABLE IF NOT EXISTS telegram_outbox (
    id            UUID PRIMARY KEY,
    chat_id       BIGINT NOT NULL,
    text          TEXT NOT NULL,
    source        TEXT NOT NULL,
    status        SMALLINT NOT NULL DEFAULT 0,
    error_message TEXT,
    sent_at       TIMESTAMPTZ,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_telegram_outbox_status ON telegram_outbox (status);
CREATE INDEX IF NOT EXISTS idx_telegram_outbox_created ON telegram_outbox (created_at);
CREATE INDEX IF NOT EXISTS idx_telegram_outbox_chat_status ON telegram_outbox (chat_id, status);
