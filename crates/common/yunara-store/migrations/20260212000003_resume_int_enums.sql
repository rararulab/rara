-- Resume domain: switch enum columns to SMALLINT codes.
--
-- Codes (aligned with domain `#[repr(u8)]`):
-- resume_source: manual=0, ai_generated=1, optimized=2

ALTER TABLE resume
    ALTER COLUMN source DROP DEFAULT;

ALTER TABLE resume
    ALTER COLUMN source TYPE SMALLINT USING (
        CASE source
            WHEN 'manual' THEN 0
            WHEN 'ai_generated' THEN 1
            WHEN 'optimized' THEN 2
        END
    );

ALTER TABLE resume
    ALTER COLUMN source SET DEFAULT 0;

