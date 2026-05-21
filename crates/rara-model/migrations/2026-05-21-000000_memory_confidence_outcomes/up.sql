-- memory confidence + outcome feedback storage primitives.
--
-- Adds a `confidence REAL` column to `memory_items` (NOT NULL DEFAULT 1.0
-- so existing rows backfill to fully trusted) and introduces an
-- append-only `memory_outcomes` audit table that records every feedback
-- event against a memory item.
--
-- The update logic, the `commit_outcome` API, and retrieval re-ranking
-- all land in issue #2112; this migration is the storage seam only.
--
-- The `outcome_kind` column is intentionally not constrained by a
-- `CHECK` — the closed `OutcomeKind` enum in the kernel is the single
-- source of truth and is enforced in Rust before insert. Adding a
-- `CHECK` here would force a migration every time we add a new outcome
-- kind.

ALTER TABLE memory_items
    ADD COLUMN confidence REAL NOT NULL DEFAULT 1.0;

CREATE TABLE memory_outcomes (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    item_id       INTEGER NOT NULL REFERENCES memory_items(id) ON DELETE CASCADE,
    outcome_kind  TEXT NOT NULL,
    tape_entry_id INTEGER,
    created_at    TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX idx_memory_outcomes_item_id ON memory_outcomes(item_id);
