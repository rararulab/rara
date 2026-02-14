-- Chat sessions and channel bindings.
-- Messages are stored as append-only JSONL files on disk, not in the database.

--------------------------------------------------------------------------------
-- chat_session: conversation session metadata
--------------------------------------------------------------------------------

CREATE TABLE chat_session (
    key           TEXT PRIMARY KEY,
    title         TEXT,
    model         TEXT,
    system_prompt TEXT,
    message_count BIGINT NOT NULL DEFAULT 0,
    preview       TEXT,
    metadata      JSONB,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_chat_session_updated_at ON chat_session (updated_at DESC);

CREATE TRIGGER set_chat_session_updated_at BEFORE UPDATE ON chat_session
    FOR EACH ROW EXECUTE FUNCTION trigger_set_updated_at();

--------------------------------------------------------------------------------
-- channel_binding: maps external channels to session keys
--------------------------------------------------------------------------------

CREATE TABLE channel_binding (
    channel_type TEXT NOT NULL,
    account      TEXT NOT NULL,
    chat_id      TEXT NOT NULL,
    session_key  TEXT NOT NULL REFERENCES chat_session(key) ON DELETE CASCADE,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (channel_type, account, chat_id)
);

CREATE TRIGGER set_channel_binding_updated_at BEFORE UPDATE ON channel_binding
    FOR EACH ROW EXECUTE FUNCTION trigger_set_updated_at();
