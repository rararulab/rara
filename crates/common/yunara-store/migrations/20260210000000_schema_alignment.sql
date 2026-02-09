-- Schema alignment: add priority/tags to application, add context fields to interview_plan.

-- Add priority enum and columns to application
CREATE TYPE application_priority AS ENUM ('low', 'medium', 'high', 'critical');
ALTER TABLE application
    ADD COLUMN tags TEXT[] NOT NULL DEFAULT '{}',
    ADD COLUMN priority application_priority NOT NULL DEFAULT 'medium';

-- Add fields to interview_plan for domain alignment
ALTER TABLE interview_plan
    ADD COLUMN company TEXT NOT NULL DEFAULT '',
    ADD COLUMN position TEXT NOT NULL DEFAULT '',
    ADD COLUMN job_description TEXT,
    ADD COLUMN round TEXT NOT NULL DEFAULT 'technical';
