-- Drop legacy scheduler tables. The scheduler subsystem was removed in
-- commit cd7b1f85 (refactor: remove legacy proactive agent and agent
-- scheduler); no Rust code references these tables anymore.

DROP INDEX IF EXISTS idx_task_run_history_started_at;
DROP INDEX IF EXISTS idx_task_run_history_task_id;
DROP TABLE IF EXISTS task_run_history;

DROP TRIGGER IF EXISTS set_scheduler_task_updated_at;
DROP INDEX IF EXISTS idx_scheduler_task_enabled;
DROP INDEX IF EXISTS idx_scheduler_task_name;
DROP TABLE IF EXISTS scheduler_task;
