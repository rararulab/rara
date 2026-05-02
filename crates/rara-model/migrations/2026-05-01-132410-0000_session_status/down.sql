-- Revert issue #2043: drop the partial index first so the column DROP
-- does not trip a "indexed column" error, then drop the column. SQLite
-- 3.35+ supports `ALTER TABLE ... DROP COLUMN`; the bundled
-- `libsqlite3-sys` (>= 3.49) is well past that floor, so the operation
-- is a single statement instead of a table rebuild.

DROP INDEX IF EXISTS idx_sessions_status_updated_at;

ALTER TABLE sessions
    DROP COLUMN status;
