-- Restore settings rows back to kv_table from the latest snapshot.
INSERT OR REPLACE INTO kv_table (key, value)
SELECT sv.key, sv.value
FROM settings_version sv
INNER JOIN (
    SELECT key, MAX(version) AS max_ver
    FROM settings_version
    GROUP BY key
) latest ON sv.key = latest.key AND sv.version = latest.max_ver
WHERE sv.value IS NOT NULL;

DROP INDEX IF EXISTS idx_settings_version_key;
DROP TABLE IF EXISTS settings_version;
DROP TABLE IF EXISTS settings_version_counter;
