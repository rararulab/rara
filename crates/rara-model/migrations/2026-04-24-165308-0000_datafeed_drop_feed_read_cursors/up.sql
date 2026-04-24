-- Drop the feed_read_cursors table. The FeedStore::mark_read and
-- FeedStore::unread_count methods that backed this table were hollow
-- (zero callers) and have been removed; see issue #1739.
DROP TABLE feed_read_cursors;
