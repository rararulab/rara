-- notification_log was used as delivery state storage.
-- Delivery state is now owned by pgmq queue semantics, so this table is removed.

DROP TABLE IF EXISTS notification_log;
