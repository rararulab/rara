-- Skill metadata cache for fast startup.
-- Filesystem remains the source of truth; DB is a cache layer.
-- source SMALLINT: project=0, personal=1, plugin=2, registry=3

CREATE TABLE IF NOT EXISTS skill_cache (
  name          TEXT PRIMARY KEY,
  description   TEXT NOT NULL DEFAULT '',
  homepage      TEXT,
  license       TEXT,
  compatibility TEXT,
  allowed_tools TEXT[] NOT NULL DEFAULT '{}',
  dockerfile    TEXT,
  requires      JSONB NOT NULL DEFAULT '{}',
  path          TEXT NOT NULL,
  source        SMALLINT NOT NULL DEFAULT 0,
  content_hash  TEXT NOT NULL,
  cached_at     TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
