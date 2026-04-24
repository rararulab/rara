-- Rename feed_events table to data_feed_events for naming consistency with
-- the data_feeds config table. Both tables belong to the data-feed subsystem.
ALTER TABLE feed_events RENAME TO data_feed_events;

-- Rename associated indexes so their names reflect the new table name.
ALTER INDEX idx_feed_events_source RENAME TO idx_data_feed_events_source;
ALTER INDEX idx_feed_events_received RENAME TO idx_data_feed_events_received;
