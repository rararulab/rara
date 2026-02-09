-- Add priority and max_retries to notification_log
CREATE TYPE notification_priority AS ENUM ('low', 'normal', 'high', 'urgent');
ALTER TABLE notification_log
    ADD COLUMN priority notification_priority NOT NULL DEFAULT 'normal',
    ADD COLUMN max_retries INTEGER NOT NULL DEFAULT 3;
