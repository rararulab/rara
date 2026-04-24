CREATE TABLE IF NOT EXISTS data_feeds (
    id          TEXT PRIMARY KEY NOT NULL,
    name        TEXT NOT NULL UNIQUE,
    feed_type   TEXT NOT NULL,
    tags        TEXT NOT NULL DEFAULT '[]',
    transport   TEXT NOT NULL DEFAULT '{}',
    auth        TEXT,
    enabled     INTEGER NOT NULL DEFAULT 1,
    status      TEXT NOT NULL DEFAULT 'idle',
    last_error  TEXT,
    created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX idx_data_feeds_name ON data_feeds(name);
CREATE INDEX idx_data_feeds_type ON data_feeds(feed_type);
