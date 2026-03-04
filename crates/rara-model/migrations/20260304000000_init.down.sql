-- Reverse of init: drop all tables in dependency order.

-- Triggers are dropped automatically with their tables in SQLite.

DROP TABLE IF EXISTS memory_embedding_cache;
DROP TABLE IF EXISTS memory_chunks;
DROP TABLE IF EXISTS memory_files;
DROP TABLE IF EXISTS task_run_history;
DROP TABLE IF EXISTS scheduler_task;
DROP TABLE IF EXISTS credential_store;
DROP TABLE IF EXISTS link_codes;
DROP TABLE IF EXISTS invite_codes;
DROP TABLE IF EXISTS telegram_contact;
DROP TABLE IF EXISTS coding_task;
DROP TABLE IF EXISTS skill_cache;
DROP TABLE IF EXISTS kernel_outbox;
DROP TABLE IF EXISTS kernel_audit_events;
DROP TABLE IF EXISTS user_platform_identities;
DROP TABLE IF EXISTS kernel_users;
DROP TABLE IF EXISTS channel_binding;
DROP TABLE IF EXISTS chat_session;
DROP TABLE IF EXISTS kv_table;
