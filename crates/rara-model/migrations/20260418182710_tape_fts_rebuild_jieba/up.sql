-- Rebuild tape_fts to re-tokenize existing entries with jieba pre-segmented
-- content. The FTS table is a derived index — source of truth is JSONL, and
-- TapeService::backfill_fts() will repopulate on next search.
DROP TABLE IF EXISTS tape_fts;
DELETE FROM tape_fts_meta;

CREATE VIRTUAL TABLE tape_fts USING fts5(
    content,
    tape_name UNINDEXED,
    entry_kind UNINDEXED,
    entry_id UNINDEXED,
    session_key UNINDEXED,
    tokenize = 'unicode61 remove_diacritics 2'
);
