DROP TABLE IF EXISTS link_codes;
DROP TABLE IF EXISTS invite_codes;
ALTER TABLE kernel_users DROP COLUMN IF EXISTS password_hash;
