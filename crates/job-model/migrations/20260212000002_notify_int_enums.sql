-- Notify domain: switch enum columns to SMALLINT codes.
--
-- Codes (aligned with domain `#[repr(u8)]`):
-- notification_channel: telegram=0, email=1, webhook=2 (other -> webhook=2)
-- notification_status: pending=0, sent=1, failed=2, retrying=3
-- notification_priority: low=0, normal=1, high=2, urgent=3

ALTER TABLE notification_log
    ALTER COLUMN status DROP DEFAULT,
    ALTER COLUMN priority DROP DEFAULT;

ALTER TABLE notification_log
    ALTER COLUMN channel TYPE SMALLINT USING (
        CASE channel
            WHEN 'telegram' THEN 0
            WHEN 'email' THEN 1
            WHEN 'webhook' THEN 2
            WHEN 'other' THEN 2
        END
    ),
    ALTER COLUMN status TYPE SMALLINT USING (
        CASE status
            WHEN 'pending' THEN 0
            WHEN 'sent' THEN 1
            WHEN 'failed' THEN 2
            WHEN 'retrying' THEN 3
        END
    ),
    ALTER COLUMN priority TYPE SMALLINT USING (
        CASE priority
            WHEN 'low' THEN 0
            WHEN 'normal' THEN 1
            WHEN 'high' THEN 2
            WHEN 'urgent' THEN 3
        END
    );

ALTER TABLE notification_log
    ALTER COLUMN status SET DEFAULT 0,
    ALTER COLUMN priority SET DEFAULT 1;

