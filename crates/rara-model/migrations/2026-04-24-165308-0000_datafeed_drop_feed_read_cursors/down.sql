-- Recreate the feed_read_cursors table with the exact schema from the
-- init migration baseline (crates/rara-model/migrations/20260304000000_init/up.sql).
CREATE TABLE feed_read_cursors (
    subscriber_id TEXT NOT NULL,
    source_name   TEXT NOT NULL,
    last_read_id  TEXT NOT NULL,
    updated_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (subscriber_id, source_name)
);
