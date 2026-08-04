CREATE TABLE devices_next (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id INTEGER NOT NULL REFERENCES accounts(id),
    device_did TEXT NOT NULL,
    attachment_id TEXT NOT NULL UNIQUE,
    delegation_cid TEXT NOT NULL UNIQUE,
    delegation_hex TEXT,
    name TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'active',
    created_at INTEGER NOT NULL
);

INSERT INTO devices_next (
    id, account_id, device_did, attachment_id, delegation_cid,
    delegation_hex, name, status, created_at
)
SELECT id, account_id, device_did, delegation_cid, delegation_cid,
       delegation_hex, name, status, created_at
FROM devices;

DROP TABLE devices;
ALTER TABLE devices_next RENAME TO devices;
CREATE INDEX devices_account ON devices(account_id);
CREATE UNIQUE INDEX devices_one_active_did
    ON devices(device_did) WHERE status = 'active';

-- Completion is durable and separate from activation.  Existing pending
-- requests receive NULL lifecycle fields and retain their original expiry.
ALTER TABLE link_requests ADD COLUMN account_id INTEGER REFERENCES accounts(id);
ALTER TABLE link_requests ADD COLUMN attachment_id TEXT;
ALTER TABLE link_requests ADD COLUMN delegation_cid TEXT;
ALTER TABLE link_requests ADD COLUMN completed_at INTEGER;
ALTER TABLE link_requests ADD COLUMN activated_at INTEGER;
ALTER TABLE link_requests ADD COLUMN cancelled_at INTEGER;
CREATE UNIQUE INDEX link_attachment
    ON link_requests(attachment_id) WHERE attachment_id IS NOT NULL;
