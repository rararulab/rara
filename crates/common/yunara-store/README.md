# yunara-store

Database layer for Yunara. Currently uses SQLite (via `sqlx`) and is planned to migrate to PostgreSQL for the backend service baseline (see repo issue #17).

Provides `DBStore` for database connections and a key-value store
extension (`KVStoreExt`) for application identifiers and settings.
