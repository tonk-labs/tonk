-- Capability-authorized hosted-space deletion lifecycle. The provider is
-- billing/discovery metadata only; destructive authority is the registered
-- direct delegation CID and its verification mode.
ALTER TABLE consumer ADD COLUMN deletion_grant_cid TEXT;
ALTER TABLE consumer ADD COLUMN deletion_grant_kind TEXT
  CHECK (deletion_grant_kind IN ('exact', 'legacy-direct'));
ALTER TABLE consumer ADD COLUMN deletion_state TEXT NOT NULL DEFAULT 'active'
  CHECK (deletion_state IN ('active', 'deleting', 'deleted'));
ALTER TABLE consumer ADD COLUMN deleted_at INTEGER;
-- Immutable creator/provider identity used only to enumerate deletion work.
-- Authorization still comes exclusively from the registered proof.
ALTER TABLE consumer ADD COLUMN owner TEXT;
UPDATE consumer SET owner = provider WHERE owner IS NULL;
CREATE INDEX consumer_owner ON consumer(owner);
