-- Typst compilation service tables.

--------------------------------------------------------------------------------
-- typst_project: top-level project container
--------------------------------------------------------------------------------

CREATE TABLE typst_project (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name        TEXT NOT NULL,
    description TEXT,
    main_file   TEXT NOT NULL DEFAULT 'main.typ',
    resume_id   UUID,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TRIGGER set_typst_project_updated_at BEFORE UPDATE ON typst_project
    FOR EACH ROW EXECUTE FUNCTION trigger_set_updated_at();

--------------------------------------------------------------------------------
-- typst_file: source files belonging to a project
--------------------------------------------------------------------------------

CREATE TABLE typst_file (
    id         UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id UUID NOT NULL REFERENCES typst_project(id) ON DELETE CASCADE,
    path       TEXT NOT NULL,
    content    TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (project_id, path)
);

CREATE INDEX idx_typst_file_project_id ON typst_file (project_id);

CREATE TRIGGER set_typst_file_updated_at BEFORE UPDATE ON typst_file
    FOR EACH ROW EXECUTE FUNCTION trigger_set_updated_at();

--------------------------------------------------------------------------------
-- typst_render: compilation output records
--------------------------------------------------------------------------------

CREATE TABLE typst_render (
    id             UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id     UUID NOT NULL REFERENCES typst_project(id) ON DELETE CASCADE,
    pdf_object_key TEXT NOT NULL,
    source_hash    TEXT NOT NULL,
    page_count     INT NOT NULL DEFAULT 0,
    file_size      BIGINT NOT NULL DEFAULT 0,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_typst_render_project_id ON typst_render (project_id);
CREATE INDEX idx_typst_render_source_hash ON typst_render (project_id, source_hash);
