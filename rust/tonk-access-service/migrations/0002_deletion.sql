-- Capability-authorized hosted-space deletion lifecycle. The provider is
-- billing/discovery metadata only; destructive authority is the registered
-- direct delegation CID and its verification mode.
ALTER TABLE subscription ADD COLUMN deletion_grant_cid TEXT;
ALTER TABLE subscription ADD COLUMN deletion_grant_kind TEXT
  CHECK (deletion_grant_kind IN ('exact', 'legacy-direct'));
ALTER TABLE subscription ADD COLUMN deletion_state TEXT NOT NULL DEFAULT 'active'
  CHECK (deletion_state IN ('active', 'deleting', 'deleted'));
ALTER TABLE subscription ADD COLUMN deleted_at INTEGER;
-- Immutable creator/provider identity used only to enumerate deletion work.
-- Authorization still comes exclusively from the registered proof.
ALTER TABLE subscription ADD COLUMN owner TEXT;
UPDATE subscription SET owner = provider WHERE owner IS NULL;
CREATE INDEX subscription_owner ON subscription(owner);
