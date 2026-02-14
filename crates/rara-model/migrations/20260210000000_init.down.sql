-- Revert consolidated init schema.

DROP TABLE IF EXISTS task_run_history;
DROP TABLE IF EXISTS scheduler_task;
DROP TABLE IF EXISTS metrics_snapshot;
DROP TABLE IF EXISTS notification_log;
DROP TABLE IF EXISTS ai_run;
DROP TABLE IF EXISTS prompt_template;
DROP TABLE IF EXISTS interview_plan;
DROP TABLE IF EXISTS application_status_history;
DROP TABLE IF EXISTS application;
DROP TABLE IF EXISTS resume;
DROP TABLE IF EXISTS job;
DROP TABLE IF EXISTS kv_table;

DROP FUNCTION IF EXISTS trigger_set_updated_at();

DROP TYPE IF EXISTS ai_model_provider;
DROP TYPE IF EXISTS prompt_kind;
DROP TYPE IF EXISTS job_status;
