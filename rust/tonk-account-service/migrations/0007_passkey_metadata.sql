-- Passkey ceremony facts recorded by Tonk at credential creation time.
-- Nullable because accounts created before this migration have no reliable
-- source for either value and must not be backfilled from account/device dates.
ALTER TABLE accounts ADD COLUMN passkey_created_at INTEGER;
ALTER TABLE accounts ADD COLUMN passkey_created_on TEXT;
