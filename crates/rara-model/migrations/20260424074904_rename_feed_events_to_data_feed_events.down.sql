-- Inverse of the rename: restore the original feed_events table and index names.
ALTER INDEX idx_data_feed_events_received RENAME TO idx_feed_events_received;
ALTER INDEX idx_data_feed_events_source RENAME TO idx_feed_events_source;
ALTER TABLE data_feed_events RENAME TO feed_events;
