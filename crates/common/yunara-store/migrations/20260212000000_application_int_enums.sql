-- Application domain: switch enum columns to SMALLINT codes.
--
-- Codes (aligned with domain `#[repr(u8)]`):
-- application_channel: direct=0, referral=1, linkedin=2, email=3, other=4
-- application_status: draft=0, submitted=1, in_progress=2, interviewing=3,
--                     offered=4, rejected=5, accepted=6, withdrawn=7
-- application_priority: low=0, medium=1, high=2, critical=3

ALTER TABLE application
    ALTER COLUMN channel DROP DEFAULT,
    ALTER COLUMN status DROP DEFAULT,
    ALTER COLUMN priority DROP DEFAULT;

ALTER TABLE application
    ALTER COLUMN channel TYPE SMALLINT USING (
        CASE channel
            WHEN 'direct' THEN 0
            WHEN 'referral' THEN 1
            WHEN 'linkedin' THEN 2
            WHEN 'email' THEN 3
            WHEN 'other' THEN 4
        END
    ),
    ALTER COLUMN status TYPE SMALLINT USING (
        CASE status
            WHEN 'draft' THEN 0
            WHEN 'submitted' THEN 1
            WHEN 'in_progress' THEN 2
            WHEN 'interviewing' THEN 3
            WHEN 'offered' THEN 4
            WHEN 'rejected' THEN 5
            WHEN 'accepted' THEN 6
            WHEN 'withdrawn' THEN 7
        END
    ),
    ALTER COLUMN priority TYPE SMALLINT USING (
        CASE priority
            WHEN 'low' THEN 0
            WHEN 'medium' THEN 1
            WHEN 'high' THEN 2
            WHEN 'critical' THEN 3
        END
    );

ALTER TABLE application
    ALTER COLUMN channel SET DEFAULT 0,
    ALTER COLUMN status SET DEFAULT 0,
    ALTER COLUMN priority SET DEFAULT 1;

ALTER TABLE application_status_history
    ALTER COLUMN from_status TYPE SMALLINT USING (
        CASE from_status
            WHEN 'draft' THEN 0
            WHEN 'submitted' THEN 1
            WHEN 'in_progress' THEN 2
            WHEN 'interviewing' THEN 3
            WHEN 'offered' THEN 4
            WHEN 'rejected' THEN 5
            WHEN 'accepted' THEN 6
            WHEN 'withdrawn' THEN 7
        END
    ),
    ALTER COLUMN to_status TYPE SMALLINT USING (
        CASE to_status
            WHEN 'draft' THEN 0
            WHEN 'submitted' THEN 1
            WHEN 'in_progress' THEN 2
            WHEN 'interviewing' THEN 3
            WHEN 'offered' THEN 4
            WHEN 'rejected' THEN 5
            WHEN 'accepted' THEN 6
            WHEN 'withdrawn' THEN 7
        END
    );

