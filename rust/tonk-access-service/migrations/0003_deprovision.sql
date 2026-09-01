-- Deletion authority is the owning customer's own delegation chain,
-- invoked as /provider/remove — the reverse of /provider/add. The
-- registered-grant columns from 0002 carried per-space /space/delete
-- artifacts and are retired; the denial-first deletion lifecycle
-- (deletion_state, deleted_at) and the owner index stay.
ALTER TABLE consumer DROP COLUMN deletion_grant_cid;
ALTER TABLE consumer DROP COLUMN deletion_grant_kind;
