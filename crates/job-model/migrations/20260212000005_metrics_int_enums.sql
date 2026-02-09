-- Metrics domain: switch period enum column to SMALLINT codes.
--
-- Codes (aligned with domain `#[repr(u8)]`):
-- metrics_period: daily=0, weekly=1, monthly=2

ALTER TABLE metrics_snapshot
    ALTER COLUMN period TYPE SMALLINT USING (
        CASE period
            WHEN 'daily' THEN 0
            WHEN 'weekly' THEN 1
            WHEN 'monthly' THEN 2
        END
    );
