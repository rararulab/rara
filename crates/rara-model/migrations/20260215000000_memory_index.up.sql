-- Memory index tables for agent long-term memory.

CREATE TABLE IF NOT EXISTS memory_files (
  id BIGSERIAL PRIMARY KEY,
  path TEXT NOT NULL UNIQUE,
  hash TEXT NOT NULL,
  mtime BIGINT NOT NULL,
  size BIGINT NOT NULL,
  updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS memory_chunks (
  id BIGSERIAL PRIMARY KEY,
  file_id BIGINT NOT NULL REFERENCES memory_files(id) ON DELETE CASCADE,
  chunk_index BIGINT NOT NULL,
  content TEXT NOT NULL,
  embedding BYTEA,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  UNIQUE(file_id, chunk_index)
);

CREATE INDEX IF NOT EXISTS idx_memory_chunks_file_idx
  ON memory_chunks(file_id, chunk_index);

CREATE INDEX IF NOT EXISTS idx_memory_chunks_content_tsv
  ON memory_chunks USING GIN (to_tsvector('simple', content));

CREATE TABLE IF NOT EXISTS memory_embedding_cache (
  id BIGSERIAL PRIMARY KEY,
  provider TEXT NOT NULL,
  model TEXT NOT NULL,
  text_hash TEXT NOT NULL,
  dim INTEGER NOT NULL,
  embedding BYTEA NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  UNIQUE(provider, model, text_hash)
);
