-- Resume version management: add parent linkage, customization, and tags.

ALTER TABLE resume
    ADD COLUMN title TEXT NOT NULL DEFAULT '',
    ADD COLUMN parent_resume_id UUID REFERENCES resume(id),
    ADD COLUMN customization_notes TEXT,
    ADD COLUMN tags TEXT[] NOT NULL DEFAULT '{}';

CREATE INDEX idx_resume_parent ON resume (parent_resume_id);
CREATE INDEX idx_resume_is_deleted ON resume (is_deleted) WHERE is_deleted = FALSE;
