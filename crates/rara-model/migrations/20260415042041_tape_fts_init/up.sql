-- FTS5 full-text index for tape-search.
-- This is a derived index — can always be rebuilt from JSONL tape files.
CREATE VIRTUAL TABLE IF NOT EXISTS tape_fts USING fts5(
    content,
    tape_name UNINDEXED,
    entry_kind UNINDEXED,
    entry_id UNINDEXED,
    session_key UNINDEXED,
    tokenize = 'unicode61 remove_diacritics 2'
);

-- Tracks the high-water mark per tape for incremental indexing.
CREATE TABLE IF NOT EXISTS tape_fts_meta (
    tape_name TEXT PRIMARY KEY,
    last_indexed_id INTEGER NOT NULL DEFAULT 0
);
