-- settings_version: MVCC append-only log — the SOLE storage for settings.
-- Each mutation (set/delete) appends a row with a monotonically increasing
-- global version. value=NULL is a tombstone (key deleted at this version).
-- Current state = snapshot at max version. kv_table no longer stores settings.
--------------------------------------------------------------------------------

CREATE TABLE settings_version (
    version    INTEGER NOT NULL,
    key        TEXT    NOT NULL,
    value      TEXT,                -- NULL = tombstone (deleted)
    changed_at TEXT    NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (version, key)
);

-- Fast snapshot queries: "latest version per key where version <= N"
CREATE INDEX idx_settings_version_key ON settings_version (key, version);

-- Global version counter. Single row, updated via UPDATE ... RETURNING.
CREATE TABLE settings_version_counter (
    id      INTEGER PRIMARY KEY CHECK (id = 1),
    current INTEGER NOT NULL DEFAULT 0
);

INSERT INTO settings_version_counter (id, current) VALUES (1, 0);

-- Seed version 0: migrate all existing settings from kv_table as baseline.
INSERT INTO settings_version (version, key, value)
SELECT 0, key, value FROM kv_table WHERE key LIKE 'settings.%';

-- Remove settings rows from kv_table — settings_version is now authoritative.
DELETE FROM kv_table WHERE key LIKE 'settings.%';
