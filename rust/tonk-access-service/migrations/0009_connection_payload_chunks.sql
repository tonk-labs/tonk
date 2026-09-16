-- D1 caps an individual string/BLOB/row at 2,000,000 bytes. Keep bounded
-- chunks beside their immutable parent; adapters commit the whole batch.
ALTER TABLE connection_delivery ADD COLUMN payload_size INTEGER NOT NULL DEFAULT 0;
ALTER TABLE connection_addition ADD COLUMN payload_size INTEGER NOT NULL DEFAULT 0;
UPDATE connection_delivery SET payload_size=LENGTH(approval_hex);
UPDATE connection_addition SET payload_size=LENGTH(addition_hex);
CREATE TABLE connection_delivery_chunk (
    request_hash TEXT NOT NULL REFERENCES connection_delivery(request_hash) ON DELETE CASCADE,
    ordinal INTEGER NOT NULL CHECK(ordinal>=0 AND ordinal<16),
    content TEXT NOT NULL CHECK(LENGTH(content)<=524288),
    PRIMARY KEY(request_hash,ordinal)
);
CREATE TABLE connection_addition_chunk (
    delivery_id TEXT NOT NULL REFERENCES connection_addition(delivery_id) ON DELETE CASCADE,
    ordinal INTEGER NOT NULL CHECK(ordinal>=0 AND ordinal<16),
    content TEXT NOT NULL CHECK(LENGTH(content)<=524288),
    PRIMARY KEY(delivery_id,ordinal)
);
