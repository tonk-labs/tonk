ALTER TABLE accounts ADD COLUMN repository_descriptor BLOB;
ALTER TABLE link_requests ADD COLUMN descriptor_hex TEXT;
