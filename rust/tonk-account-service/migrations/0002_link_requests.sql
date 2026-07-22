CREATE TABLE link_requests (
    token_hash TEXT PRIMARY KEY,
    device_did TEXT NOT NULL,
    device_name TEXT NOT NULL,
    delegation_hex TEXT,
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    consumed_at INTEGER
);
