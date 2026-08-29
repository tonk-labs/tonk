-- Capability-authorized hosted-space deletion lifecycle. The provider is
-- billing/discovery metadata only; destructive authority is the registered
-- direct delegation CID and its verification mode.
ALTER TABLE subscription ADD COLUMN deletion_grant_cid TEXT;
ALTER TABLE subscription ADD COLUMN deletion_grant_kind TEXT
  CHECK (deletion_grant_kind IN ('exact', 'legacy-direct'));
ALTER TABLE subscription ADD COLUMN deletion_state TEXT NOT NULL DEFAULT 'active'
  CHECK (deletion_state IN ('active', 'deleting', 'deleted'));
ALTER TABLE subscription ADD COLUMN deleted_at INTEGER;
-- Deletion work is enumerated by provider. Nothing transfers a space to
-- a different payer, so the customer who pays for one is the account
-- whose data it is; a separate `owner` would hold the same value on
-- every write. It arrives with the `space` table, when a space may have
-- several providers and ownership stops being derivable from any of
-- them.
CREATE INDEX subscription_provider ON subscription(provider);
