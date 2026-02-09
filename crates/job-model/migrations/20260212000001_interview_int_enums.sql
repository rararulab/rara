-- Interview domain: switch enum columns to SMALLINT codes.
--
-- Codes (aligned with domain `#[repr(u8)]`):
-- interview_task_status: pending=0, in_progress=1, completed=2, skipped=3

ALTER TABLE interview_plan
    ALTER COLUMN task_status DROP DEFAULT;

ALTER TABLE interview_plan
    ALTER COLUMN task_status TYPE SMALLINT USING (
        CASE task_status
            WHEN 'pending' THEN 0
            WHEN 'in_progress' THEN 1
            WHEN 'completed' THEN 2
            WHEN 'skipped' THEN 3
        END
    );

ALTER TABLE interview_plan
    ALTER COLUMN task_status SET DEFAULT 0;

