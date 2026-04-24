-- Inverse of the rename: restore the original feed_events table and index names.
-- SQLite has no ALTER INDEX; use DROP + CREATE instead.
DROP INDEX idx_data_feed_events_source;
DROP INDEX idx_data_feed_events_received;

ALTER TABLE data_feed_events RENAME TO feed_events;

CREATE INDEX idx_feed_events_source ON feed_events(source_name);
CREATE INDEX idx_feed_events_received ON feed_events(received_at);
