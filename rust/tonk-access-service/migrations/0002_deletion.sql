-- Capability-authorized hosted-space deletion lifecycle. The provider is
-- billing/discovery metadata only; destructive authority is the registered
-- direct delegation CID and its verification mode.
ALTER TABLE subscription ADD COLUMN deletion_grant_cid TEXT;
ALTER TABLE subscription ADD COLUMN deletion_grant_kind TEXT
  CHECK (deletion_grant_kind IN ('exact', 'legacy-direct'));
-- When deletion began. Null means it has not, and the row disappears
-- when it finishes, so there is no third state to keep in step.
--
-- Purging R2 is neither atomic nor guaranteed to finish in one attempt,
-- so service has to stop before the objects do: a client must not read a
-- half-purged space. Setting this is what stops it, and the write is a
-- compare-and-set, so two concurrent deletions cannot both start one.
ALTER TABLE subscription ADD COLUMN deleted_at INTEGER;
-- Deletion work is enumerated by provider. Nothing transfers a space to
-- a different payer, so the customer who pays for one is the account
-- whose data it is; a separate `owner` would hold the same value on
-- every write. It arrives with the `space` table, when a space may have
-- several providers and ownership stops being derivable from any of
-- them.
CREATE INDEX subscription_provider ON subscription(provider);
