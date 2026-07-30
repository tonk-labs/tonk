CREATE TABLE devices_canonical (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id INTEGER NOT NULL REFERENCES accounts(id),
    device_did TEXT NOT NULL UNIQUE,
    delegation_cid TEXT NOT NULL,
    delegation_hex TEXT,
    name TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'active',
    created_at INTEGER NOT NULL
);

INSERT INTO devices_canonical (
    id, account_id, device_did, delegation_cid, delegation_hex, name, status, created_at
)
SELECT id, account_id, device_did, delegation_cid, delegation_hex, name, status, created_at
FROM devices;

DROP TABLE devices;
ALTER TABLE devices_canonical RENAME TO devices;
CREATE INDEX devices_account ON devices(account_id);
