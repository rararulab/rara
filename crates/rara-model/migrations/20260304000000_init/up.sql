-- Baseline init migration for Rara (squashed from 7 incremental migrations).
--
-- Conventions:
-- * All UUID/ULID columns are TEXT (generated Rust-side).
-- * All timestamp columns are TEXT in ISO 8601 format.
-- * All JSON columns are TEXT (JSON strings).
-- * Boolean columns are INTEGER (0/1).

--------------------------------------------------------------------------------
-- kv_table: key-value storage
--------------------------------------------------------------------------------

CREATE TABLE kv_table (
    key   TEXT NOT NULL PRIMARY KEY,
    value TEXT
);

--------------------------------------------------------------------------------
-- chat_session: conversation session metadata
--------------------------------------------------------------------------------

CREATE TABLE chat_session (
    key           TEXT PRIMARY KEY,
    title         TEXT,
    model         TEXT,
    system_prompt TEXT,
    message_count INTEGER NOT NULL DEFAULT 0,
    preview       TEXT,
    metadata      TEXT,
    created_at    TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at    TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX idx_chat_session_updated_at ON chat_session (updated_at DESC);

CREATE TRIGGER set_chat_session_updated_at AFTER UPDATE ON chat_session
BEGIN
    UPDATE chat_session SET updated_at = datetime('now') WHERE key = NEW.key;
END;

--------------------------------------------------------------------------------
-- channel_binding: maps external channels to session keys
--------------------------------------------------------------------------------

CREATE TABLE channel_binding (
    channel_type TEXT NOT NULL,
    account      TEXT NOT NULL,
    chat_id      TEXT NOT NULL,
    session_key  TEXT NOT NULL REFERENCES chat_session(key) ON DELETE CASCADE,
    created_at   TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at   TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (channel_type, account, chat_id)
);

CREATE TRIGGER set_channel_binding_updated_at AFTER UPDATE ON channel_binding
BEGIN
    UPDATE channel_binding SET updated_at = datetime('now')
        WHERE channel_type = NEW.channel_type AND account = NEW.account AND chat_id = NEW.chat_id;
END;

--------------------------------------------------------------------------------
-- kernel_users: user management
--------------------------------------------------------------------------------

CREATE TABLE kernel_users (
    id            TEXT NOT NULL PRIMARY KEY,
    name          TEXT NOT NULL UNIQUE,
    role          INTEGER NOT NULL DEFAULT 2,
    permissions   TEXT NOT NULL DEFAULT '[]',
    enabled       INTEGER NOT NULL DEFAULT 1,
    created_at    TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at    TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TRIGGER set_kernel_users_updated_at AFTER UPDATE ON kernel_users
BEGIN
    UPDATE kernel_users SET updated_at = datetime('now') WHERE id = NEW.id;
END;

--------------------------------------------------------------------------------
-- kernel_audit_events: persistent audit trail
--------------------------------------------------------------------------------

CREATE TABLE kernel_audit_events (
    id          TEXT NOT NULL PRIMARY KEY,
    timestamp   TEXT NOT NULL DEFAULT (datetime('now')),
    agent_id    TEXT NOT NULL,
    session_id  TEXT NOT NULL,
    user_id     TEXT NOT NULL,
    event_type  TEXT NOT NULL,
    event_data  TEXT NOT NULL DEFAULT '{}',
    details     TEXT NOT NULL DEFAULT '{}',
    created_at  TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX idx_audit_agent_ts ON kernel_audit_events (agent_id, timestamp);
CREATE INDEX idx_audit_user_ts ON kernel_audit_events (user_id, timestamp);
CREATE INDEX idx_audit_event_type ON kernel_audit_events (event_type);

--------------------------------------------------------------------------------
-- kernel_outbox: event delivery outbox
--------------------------------------------------------------------------------

CREATE TABLE kernel_outbox (
    id           TEXT NOT NULL PRIMARY KEY,
    channel_type TEXT NOT NULL,
    target       TEXT NOT NULL,
    payload      TEXT NOT NULL,
    status       INTEGER NOT NULL DEFAULT 0,
    created_at   TEXT NOT NULL DEFAULT (datetime('now')),
    delivered_at TEXT
);

CREATE INDEX idx_outbox_pending ON kernel_outbox (status, created_at)
    WHERE status = 0;

--------------------------------------------------------------------------------
-- skill_cache: skill metadata cache for fast startup
-- source INTEGER: project=0, personal=1, plugin=2, registry=3
--------------------------------------------------------------------------------

CREATE TABLE skill_cache (
    name          TEXT PRIMARY KEY,
    description   TEXT NOT NULL DEFAULT '',
    homepage      TEXT,
    license       TEXT,
    compatibility TEXT,
    allowed_tools TEXT NOT NULL DEFAULT '[]',
    dockerfile    TEXT,
    requires      TEXT NOT NULL DEFAULT '{}',
    path          TEXT NOT NULL,
    source        INTEGER NOT NULL DEFAULT 0,
    content_hash  TEXT NOT NULL,
    cached_at     TEXT NOT NULL DEFAULT (datetime('now'))
);

--------------------------------------------------------------------------------
-- coding_task: coding task management
--------------------------------------------------------------------------------

CREATE TABLE coding_task (
    id              TEXT NOT NULL PRIMARY KEY,
    status          INTEGER NOT NULL DEFAULT 0,
    agent_type      INTEGER NOT NULL DEFAULT 0,
    repo_url        TEXT NOT NULL,
    branch          TEXT NOT NULL,
    prompt          TEXT NOT NULL,
    pr_url          TEXT,
    pr_number       INTEGER,
    session_key     TEXT,
    tmux_session    TEXT NOT NULL DEFAULT '',
    workspace_path  TEXT NOT NULL DEFAULT '',
    output          TEXT NOT NULL DEFAULT '',
    exit_code       INTEGER,
    error           TEXT,
    created_at      TEXT NOT NULL DEFAULT (datetime('now')),
    started_at      TEXT,
    completed_at    TEXT
);

CREATE INDEX idx_coding_task_status ON coding_task(status);
CREATE INDEX idx_coding_task_created ON coding_task(created_at DESC);

--------------------------------------------------------------------------------
-- credential_store: encrypted credential storage
--------------------------------------------------------------------------------

CREATE TABLE credential_store (
    service    TEXT NOT NULL,
    account    TEXT NOT NULL,
    value      TEXT NOT NULL,
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (service, account)
);

--------------------------------------------------------------------------------
-- memory_items: knowledge-layer memory entries (with embeddings)
--------------------------------------------------------------------------------

CREATE TABLE memory_items (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    username        TEXT NOT NULL,
    content         TEXT NOT NULL,
    memory_type     TEXT NOT NULL,
    category        TEXT NOT NULL,
    source_tape     TEXT,
    source_entry_id INTEGER,
    embedding       BLOB,
    created_at      TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at      TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX idx_memory_items_username ON memory_items(username);
CREATE INDEX idx_memory_items_category ON memory_items(username, category);

CREATE TRIGGER set_memory_items_updated_at AFTER UPDATE ON memory_items
BEGIN
    UPDATE memory_items SET updated_at = datetime('now') WHERE id = NEW.id;
END;

--------------------------------------------------------------------------------
-- execution_traces: JSON-serialized ExecutionTrace records per session
--------------------------------------------------------------------------------

CREATE TABLE execution_traces (
    -- ULID primary key, sortable by creation time.
    id          TEXT    PRIMARY KEY NOT NULL,
    -- Session that produced this trace.
    session_id  TEXT    NOT NULL,
    -- Full ExecutionTrace as JSON.
    trace_data  TEXT    NOT NULL,
    created_at  TEXT    NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX idx_execution_traces_session ON execution_traces(session_id);

--------------------------------------------------------------------------------
-- data_feed_events: external data ingested by the data feed subsystem
--------------------------------------------------------------------------------

CREATE TABLE data_feed_events (
    id          TEXT PRIMARY KEY NOT NULL,
    source_name TEXT NOT NULL,
    event_type  TEXT NOT NULL,
    tags        TEXT NOT NULL DEFAULT '[]',
    payload     TEXT NOT NULL DEFAULT '{}',
    received_at TEXT NOT NULL,
    created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX idx_data_feed_events_source ON data_feed_events(source_name);
CREATE INDEX idx_data_feed_events_received ON data_feed_events(received_at);

-- Per-subscriber read cursors for tracking consumption progress.
CREATE TABLE feed_read_cursors (
    subscriber_id TEXT NOT NULL,
    source_name   TEXT NOT NULL,
    last_read_id  TEXT NOT NULL,
    updated_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (subscriber_id, source_name)
);

--------------------------------------------------------------------------------
-- data_feeds: configured data feed sources
--------------------------------------------------------------------------------

CREATE TABLE data_feeds (
    id          TEXT PRIMARY KEY NOT NULL,
    name        TEXT NOT NULL UNIQUE,
    feed_type   TEXT NOT NULL,
    tags        TEXT NOT NULL DEFAULT '[]',
    transport   TEXT NOT NULL DEFAULT '{}',
    auth        TEXT,
    enabled     INTEGER NOT NULL DEFAULT 1,
    status      TEXT NOT NULL DEFAULT 'idle',
    last_error  TEXT,
    created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX idx_data_feeds_name ON data_feeds(name);
CREATE INDEX idx_data_feeds_type ON data_feeds(feed_type);

--------------------------------------------------------------------------------
-- tape_fts: FTS5 full-text index for tape-search.
--
-- This is a derived index — source of truth is the JSONL tape files, and
-- TapeService::backfill_fts() repopulates on next search. Content written
-- here is jieba pre-segmented at insert time (the `unicode61` tokenizer
-- handles whitespace splitting after Rust-side segmentation).
--------------------------------------------------------------------------------

CREATE VIRTUAL TABLE tape_fts USING fts5(
    content,
    tape_name UNINDEXED,
    entry_kind UNINDEXED,
    entry_id UNINDEXED,
    session_key UNINDEXED,
    tokenize = 'unicode61 remove_diacritics 2'
);

-- Tracks the high-water mark per tape for incremental indexing.
CREATE TABLE tape_fts_meta (
    tape_name TEXT PRIMARY KEY,
    last_indexed_id INTEGER NOT NULL DEFAULT 0
);
