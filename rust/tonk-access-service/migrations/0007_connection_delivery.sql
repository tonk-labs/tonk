-- Immutable public, signed approvals. These rows deliver capabilities; they
-- never authorize storage access. No anonymous pending requests are stored.
CREATE TABLE connection_delivery (
    request_hash TEXT PRIMARY KEY NOT NULL,
    recipient TEXT NOT NULL,
    account TEXT NOT NULL,
    approval_hex TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    approval_deadline INTEGER NOT NULL
);
CREATE INDEX connection_delivery_account ON connection_delivery(account);
