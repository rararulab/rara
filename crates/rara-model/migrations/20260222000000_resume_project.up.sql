-- Drop old resume table and related objects
DROP TABLE IF EXISTS resume CASCADE;

-- Drop old typst tables
DROP TABLE IF EXISTS typst_render CASCADE;
DROP TABLE IF EXISTS typst_project CASCADE;

-- Create new resume_project table
CREATE TABLE resume_project (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name            TEXT NOT NULL,
    git_url         TEXT NOT NULL,
    local_path      TEXT NOT NULL,
    last_synced_at  TIMESTAMPTZ,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TRIGGER set_updated_at BEFORE UPDATE ON resume_project
    FOR EACH ROW EXECUTE FUNCTION trigger_set_updated_at();
