-- Issue #2043: per-session active/archived status.
--
-- Adds a `status` column on the existing `sessions` row (Decision 3) so
-- the sidebar can hide one-off / abandoned sessions without deleting
-- their tape. The column-level CHECK constraint pins the value space to
-- the two-state taxonomy (Decision 1); a future "third state" is a
-- column rewrite, not a silent expansion.
--
-- Existing rows become `status = 'active'` (which is the correct
-- semantics — no row was archived before this change shipped). The
-- partial index `idx_sessions_status_updated_at` accelerates the
-- default-filtered list path (`status='active'` + `updated_at DESC`)
-- so adding the column does not regress the existing
-- `idx_sessions_updated_at` plan for an unfiltered scan.

ALTER TABLE sessions
    ADD COLUMN status TEXT NOT NULL DEFAULT 'active'
        CHECK (status IN ('active', 'archived'));

CREATE INDEX idx_sessions_status_updated_at
    ON sessions (status, updated_at DESC);
