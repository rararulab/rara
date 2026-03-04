-- Consolidated SQLite init migration for Rara.
-- All UUID columns are TEXT (generated Rust-side or via randomblob default).
-- All timestamp columns are TEXT in ISO 8601 format.
-- All JSON columns are TEXT (JSON strings).
-- Boolean columns are INTEGER (0/1).

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
-- scheduler_task: cron task metadata
-- last_status INTEGER: success=0, failed=1, running=2
--------------------------------------------------------------------------------

CREATE TABLE scheduler_task (
    id            TEXT NOT NULL PRIMARY KEY,
    name          TEXT NOT NULL UNIQUE,
    cron_expr     TEXT NOT NULL,
    enabled       INTEGER NOT NULL DEFAULT 1,
    last_run_at   TEXT,
    last_status   INTEGER,
    last_error    TEXT,
    run_count     INTEGER NOT NULL DEFAULT 0,
    failure_count INTEGER NOT NULL DEFAULT 0,
    is_deleted    INTEGER NOT NULL DEFAULT 0,
    deleted_at    TEXT,
    created_at    TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at    TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX idx_scheduler_task_name ON scheduler_task(name);
CREATE INDEX idx_scheduler_task_enabled ON scheduler_task(enabled)
    WHERE is_deleted = 0;

CREATE TRIGGER set_scheduler_task_updated_at AFTER UPDATE ON scheduler_task
BEGIN
    UPDATE scheduler_task SET updated_at = datetime('now') WHERE id = NEW.id;
END;

--------------------------------------------------------------------------------
-- task_run_history: scheduler execution log
-- status INTEGER: success=0, failed=1, running=2
--------------------------------------------------------------------------------

CREATE TABLE task_run_history (
    id          TEXT NOT NULL PRIMARY KEY,
    task_id     TEXT NOT NULL REFERENCES scheduler_task(id),
    status      INTEGER NOT NULL,
    started_at  TEXT NOT NULL,
    finished_at TEXT,
    duration_ms INTEGER,
    error       TEXT,
    output      TEXT,
    created_at  TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX idx_task_run_history_task_id ON task_run_history(task_id);
CREATE INDEX idx_task_run_history_started_at ON task_run_history(started_at DESC);
