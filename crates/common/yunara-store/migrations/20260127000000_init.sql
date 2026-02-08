-- Create kv_table for key-value storage
CREATE TABLE IF NOT EXISTS kv_table (
  key TEXT NOT NULL PRIMARY KEY,
  value TEXT
);
