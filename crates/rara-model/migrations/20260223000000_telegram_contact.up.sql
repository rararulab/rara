CREATE TABLE telegram_contact (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name              TEXT NOT NULL,
    telegram_username TEXT NOT NULL UNIQUE,
    chat_id           BIGINT,
    notes             TEXT,
    enabled           BOOLEAN NOT NULL DEFAULT TRUE,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TRIGGER set_updated_at BEFORE UPDATE ON telegram_contact
    FOR EACH ROW EXECUTE FUNCTION trigger_set_updated_at();
