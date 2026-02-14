-- Add git integration fields to typst_project.

ALTER TABLE typst_project ADD COLUMN git_url TEXT;
ALTER TABLE typst_project ADD COLUMN git_last_synced_at TIMESTAMPTZ;
