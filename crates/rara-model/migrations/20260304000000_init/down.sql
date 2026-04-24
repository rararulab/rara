-- Reverse of init: drop all objects in reverse dependency order.
-- Triggers are dropped automatically with their tables in SQLite.

DROP TABLE IF EXISTS tape_fts_meta;
DROP TABLE IF EXISTS tape_fts;
DROP TABLE IF EXISTS data_feeds;
DROP TABLE IF EXISTS feed_read_cursors;
DROP TABLE IF EXISTS feed_events;
DROP TABLE IF EXISTS execution_traces;
DROP TABLE IF EXISTS memory_items;
DROP TABLE IF EXISTS task_run_history;
DROP TABLE IF EXISTS scheduler_task;
DROP TABLE IF EXISTS credential_store;
DROP TABLE IF EXISTS coding_task;
DROP TABLE IF EXISTS skill_cache;
DROP TABLE IF EXISTS kernel_outbox;
DROP TABLE IF EXISTS kernel_audit_events;
DROP TABLE IF EXISTS kernel_users;
DROP TABLE IF EXISTS channel_binding;
DROP TABLE IF EXISTS chat_session;
DROP TABLE IF EXISTS kv_table;
