-- A consumer is either a user's data space or a custody namespace the
-- account provisions for its own key material. The kind lets deletion
-- tell them apart: custody namespaces never appear in a review and are
-- purged by customer finalization, last — so the deletion machinery
-- cannot destroy the account's own key custody mid-flight.
ALTER TABLE consumer ADD COLUMN kind TEXT NOT NULL DEFAULT 'space';
