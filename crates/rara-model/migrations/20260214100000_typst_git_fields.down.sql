-- Remove git integration fields from typst_project.

ALTER TABLE typst_project DROP COLUMN IF EXISTS git_last_synced_at;
ALTER TABLE typst_project DROP COLUMN IF EXISTS git_url;
