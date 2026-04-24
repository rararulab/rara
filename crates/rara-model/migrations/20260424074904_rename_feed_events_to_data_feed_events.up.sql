-- Rename feed_events table to data_feed_events for naming consistency with
-- the data_feeds config table. SQLite: ALTER TABLE RENAME is supported;
-- indexes must be DROP + CREATE because SQLite has no ALTER INDEX.
ALTER TABLE feed_events RENAME TO data_feed_events;

DROP INDEX idx_feed_events_source;
DROP INDEX idx_feed_events_received;

CREATE INDEX idx_data_feed_events_source ON data_feed_events(source_name);
CREATE INDEX idx_data_feed_events_received ON data_feed_events(received_at);
