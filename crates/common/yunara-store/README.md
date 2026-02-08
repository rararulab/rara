# yunara-store

Database layer for Yunara using PostgreSQL (via `sqlx`).

Provides `DBStore` for database connections and a key-value store
extension (`KVStoreExt`) for application identifiers and settings.

## Testing

Integration-style tests in `src/kv.rs` run only when either `YUNARA_STORE_TEST_DATABASE_URL` or `DATABASE_URL` is set.
