-- Public immutable additions addressed to an already approved terminal.
-- Sequence numbers are delivery cursors, never authorization versions.
CREATE TABLE connection_addition (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    delivery_id TEXT NOT NULL UNIQUE,
    request_hash TEXT NOT NULL REFERENCES connection_delivery(request_hash),
    recipient TEXT NOT NULL,
    account TEXT NOT NULL,
    addition_hex TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE INDEX connection_addition_recipient ON connection_addition(request_hash, recipient, sequence);
CREATE INDEX connection_addition_account ON connection_addition(account);
