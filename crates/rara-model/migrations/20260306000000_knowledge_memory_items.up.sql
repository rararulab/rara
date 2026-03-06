CREATE TABLE memory_items (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    username        TEXT NOT NULL,
    content         TEXT NOT NULL,
    memory_type     TEXT NOT NULL,
    category        TEXT NOT NULL,
    source_tape     TEXT,
    source_entry_id INTEGER,
    embedding       BLOB,
    created_at      TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at      TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX idx_memory_items_username ON memory_items(username);
CREATE INDEX idx_memory_items_category ON memory_items(username, category);

CREATE TRIGGER set_memory_items_updated_at AFTER UPDATE ON memory_items
BEGIN
    UPDATE memory_items SET updated_at = datetime('now') WHERE id = NEW.id;
END;
