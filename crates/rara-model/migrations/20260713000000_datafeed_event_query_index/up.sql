-- Optimize feed-event pages used by finance source diagnostics and event
-- inspection. These queries filter by source_name, optionally by event_type,
-- then return the latest events in a deterministic order.

CREATE INDEX idx_data_feed_events_source_received_created_id
ON data_feed_events(source_name, received_at DESC, created_at DESC, id DESC);

CREATE INDEX idx_data_feed_events_source_type_received_created_id
ON data_feed_events(source_name, event_type, received_at DESC, created_at DESC, id DESC);
