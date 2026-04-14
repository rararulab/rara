-- Feed events: external data ingested by the data feed subsystem.
CREATE TABLE IF NOT EXISTS feed_events (
    id          TEXT PRIMARY KEY NOT NULL,
    source_name TEXT NOT NULL,
    event_type  TEXT NOT NULL,
    tags        TEXT NOT NULL DEFAULT '[]',
    payload     TEXT NOT NULL DEFAULT '{}',
    received_at TEXT NOT NULL,
    created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX idx_feed_events_source ON feed_events(source_name);
CREATE INDEX idx_feed_events_received ON feed_events(received_at);

-- Per-subscriber read cursors for tracking consumption progress.
CREATE TABLE IF NOT EXISTS feed_read_cursors (
    subscriber_id TEXT NOT NULL,
    source_name   TEXT NOT NULL,
    last_read_id  TEXT NOT NULL,
    updated_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (subscriber_id, source_name)
);
