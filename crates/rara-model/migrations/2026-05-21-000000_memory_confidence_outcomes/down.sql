-- Reverse the confidence + outcomes migration.
--
-- SQLite ≥ 3.35 supports `ALTER TABLE ... DROP COLUMN`, which is the
-- approach we take here. Down-migration is best-effort: rows that have
-- accumulated a non-default `confidence` value lose that information,
-- and any `memory_outcomes` rows are deleted with their parent CASCADE
-- — which is appropriate since the upstream feature is being removed.

DROP INDEX IF EXISTS idx_memory_outcomes_item_id;
DROP TABLE IF EXISTS memory_outcomes;

ALTER TABLE memory_items DROP COLUMN confidence;
