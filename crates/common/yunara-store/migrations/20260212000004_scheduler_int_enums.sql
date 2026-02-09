-- Scheduler domain: switch enum columns to SMALLINT codes.
--
-- Codes (aligned with domain `#[repr(u8)]`):
-- task_run_status: success=0, failed=1, running=2

ALTER TABLE scheduler_task
    ALTER COLUMN last_status TYPE SMALLINT USING (
        CASE last_status
            WHEN 'success' THEN 0
            WHEN 'failed' THEN 1
            WHEN 'running' THEN 2
        END
    );

ALTER TABLE task_run_history
    ALTER COLUMN status TYPE SMALLINT USING (
        CASE status
            WHEN 'success' THEN 0
            WHEN 'failed' THEN 1
            WHEN 'running' THEN 2
        END
    );

