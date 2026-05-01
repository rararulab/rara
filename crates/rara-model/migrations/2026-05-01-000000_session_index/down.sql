-- Issue #2025: roll back the SQLite-backed session index.

DROP INDEX IF EXISTS idx_session_channel_bindings_session_key;
DROP TABLE IF EXISTS session_channel_bindings;
DROP INDEX IF EXISTS idx_sessions_updated_at;
DROP TABLE IF EXISTS sessions;
