# yunara-store

Shared diesel-async + bb8 SQLite/Postgres connection pool and a JSON
key-value store backed by the `kv_table` schema owned by `rara-model`.

Provides `DBStore` / `DieselSqlitePool` / `DieselPgPool` for database
connections and `KVStore` / `KVStoreExt` for application identifiers and
runtime settings.
