-- Drop stale AI tables that are no longer used after rig-core migration (#56, #61).
-- ai_run references prompt_template, so drop it first.
DROP TABLE IF EXISTS ai_run;
DROP TABLE IF EXISTS prompt_template;
DROP TYPE IF EXISTS ai_model_provider;
DROP TYPE IF EXISTS prompt_kind;
