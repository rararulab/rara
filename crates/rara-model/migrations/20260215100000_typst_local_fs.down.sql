-- Revert local filesystem mode: re-create typst_file table, drop local_path.

ALTER TABLE typst_project DROP COLUMN IF EXISTS local_path;

CREATE TABLE IF NOT EXISTS typst_file (
    id         UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id UUID NOT NULL REFERENCES typst_project(id) ON DELETE CASCADE,
    path       TEXT NOT NULL,
    content    TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (project_id, path)
);

CREATE INDEX IF NOT EXISTS idx_typst_file_project_id ON typst_file (project_id);
