-- Add password_hash to kernel_users
ALTER TABLE kernel_users ADD COLUMN IF NOT EXISTS password_hash TEXT;

-- Invite codes for registration
CREATE TABLE IF NOT EXISTS invite_codes (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    code TEXT NOT NULL UNIQUE,
    created_by UUID NOT NULL REFERENCES kernel_users(id),
    used_by UUID REFERENCES kernel_users(id),
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Link codes for TG binding
CREATE TABLE IF NOT EXISTS link_codes (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    code TEXT NOT NULL UNIQUE,
    user_id UUID NOT NULL REFERENCES kernel_users(id),
    direction TEXT NOT NULL CHECK (direction IN ('web_to_tg', 'tg_to_web')),
    platform_data JSONB,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
