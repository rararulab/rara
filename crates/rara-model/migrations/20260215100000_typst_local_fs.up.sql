-- Switch Typst system to local filesystem mode.
-- The typst_file table is no longer needed; file content is read from disk.

DROP TABLE IF EXISTS typst_file;

ALTER TABLE typst_project ADD COLUMN IF NOT EXISTS local_path TEXT NOT NULL DEFAULT '';
