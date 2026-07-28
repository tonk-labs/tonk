CREATE TABLE accounts (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    email TEXT NOT NULL UNIQUE,
    root_did TEXT NOT NULL UNIQUE,
    credential_id TEXT NOT NULL,
    created_at INTEGER NOT NULL
);

CREATE TABLE devices (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id INTEGER NOT NULL REFERENCES accounts(id),
    device_did TEXT NOT NULL UNIQUE,
    delegation_cid TEXT NOT NULL,
    delegation_hex TEXT NOT NULL,
    name TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'active',
    created_at INTEGER NOT NULL
);

CREATE INDEX devices_account ON devices(account_id);

CREATE TABLE email_codes (
    email TEXT PRIMARY KEY,
    code_hash TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 0
);
