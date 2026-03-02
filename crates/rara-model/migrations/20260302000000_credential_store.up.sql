CREATE TABLE IF NOT EXISTS credential_store (
    service    TEXT NOT NULL,
    account    TEXT NOT NULL,
    value      TEXT NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (service, account)
);
