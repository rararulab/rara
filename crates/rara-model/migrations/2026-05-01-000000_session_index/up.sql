-- Issue #2025: SQLite-backed session index with tape-derived state.
--
-- Replaces the JSON-file-backed `FileSessionIndex` (which had stale
-- `message_count`/`updated_at` fields and required O(N) directory scans
-- on every `GET /api/v1/chat/sessions`) with a real table indexed on
-- `updated_at DESC`. Derived state (`total_entries`, `anchors_json`,
-- `last_token_usage`, `estimated_context_tokens`,
-- `entries_since_last_anchor`) is updated synchronously on every tape
-- append by `TapeService::append`.
--
-- The legacy `chat_session` table from the squashed init migration is
-- left in place because it is not joined or queried at runtime.

CREATE TABLE sessions (
    key                          TEXT NOT NULL PRIMARY KEY,
    title                        TEXT,
    model                        TEXT,
    model_provider               TEXT,
    thinking_level               TEXT,
    system_prompt                TEXT,
    total_entries                INTEGER NOT NULL DEFAULT 0,
    preview                      TEXT,
    last_token_usage             INTEGER,
    estimated_context_tokens     INTEGER NOT NULL DEFAULT 0,
    entries_since_last_anchor    INTEGER NOT NULL DEFAULT 0,
    anchors_json                 TEXT NOT NULL DEFAULT '[]',
    metadata                     TEXT,
    created_at                   TEXT NOT NULL,
    updated_at                   TEXT NOT NULL
) WITHOUT ROWID;

CREATE INDEX idx_sessions_updated_at ON sessions (updated_at DESC);

CREATE TABLE session_channel_bindings (
    channel_type TEXT NOT NULL,
    chat_id      TEXT NOT NULL,
    thread_id    TEXT,
    session_key  TEXT NOT NULL,
    created_at   TEXT NOT NULL,
    updated_at   TEXT NOT NULL,
    PRIMARY KEY (channel_type, chat_id, thread_id)
);

CREATE INDEX idx_session_channel_bindings_session_key
    ON session_channel_bindings (session_key);
