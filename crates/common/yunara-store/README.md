# yunara-store

Shared diesel-async + bb8 SQLite connection pool and a JSON
key-value store backed by the `kv_table` schema owned by `rara-model`.

Provides `DBStore` / `DieselSqlitePool` for database connections and
`KVStore` / `KVStoreExt` for application identifiers and runtime settings.
