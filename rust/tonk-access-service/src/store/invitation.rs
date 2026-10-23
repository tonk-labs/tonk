//! Durable pending delivery and atomic claim/cancel/retry transitions.

use super::StoreError;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Stored delivery. Its email is private service data, never portable grant metadata.
#[derive(Clone, Serialize, Deserialize)]
pub struct PendingInvitation {
    /// Random 32-byte hex identifier.
    pub id: String,
    /// Authenticated sending account.
    pub sender: String,
    /// Normalized intended email.
    pub email: String,
    /// Space explicitly selected by the sender.
    pub space: String,
    /// Creation time, in seconds.
    pub created_at: u64,
    /// Latest claim time.
    pub expires_at: u64,
    /// Original signed grant, in base58.
    pub original: Option<String>,
    /// Pinned claiming account.
    pub recipient: Option<String>,
    /// Completed chain, in base58, returned identically on retries.
    pub completed: Option<String>,
    /// Successful transport acknowledgement.
    pub sent_at: Option<u64>,
    /// Whether the sender cancelled pending delivery.
    pub cancelled: u32,
    /// Bounded transport attempts.
    pub attempts: u32,
    /// Lease/cooldown before another transport attempt.
    pub next_attempt_at: u64,
}

/// Atomic operations shared by D1 and native SQLite.
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
pub trait InvitationStore {
    /// Reserve once, enforcing per-sender and per-address daily limits atomically.
    async fn reserve_invitation(&self, record: &PendingInvitation) -> Result<bool, StoreError>;
    /// Read current immutable binding and delivery state.
    async fn invitation(&self, id: &str) -> Result<Option<PendingInvitation>, StoreError>;
    /// Attach the original once. An existing different grant cannot replace it.
    async fn publish_invitation(
        &self,
        id: &str,
        original: &str,
        now: u64,
    ) -> Result<bool, StoreError>;
    /// Claim a 60-second delivery lease, at most three times within the provider's
    /// 24-hour idempotency window. Acknowledged messages never resend.
    async fn lease_invitation(&self, id: &str, now: u64) -> Result<bool, StoreError>;
    /// Record transport acknowledgement.
    async fn invitation_sent(&self, id: &str, now: u64) -> Result<(), StoreError>;
    /// Compare-and-set completion only while pending and the same email is Active.
    async fn complete_invitation(
        &self,
        id: &str,
        recipient: &str,
        completed: &str,
        now: u64,
    ) -> Result<bool, StoreError>;
    /// Cancel only before a claim has won.
    async fn cancel_invitation(&self, id: &str, sender: &str) -> Result<bool, StoreError>;
}

pub(crate) const RESERVE: &str = "INSERT OR IGNORE INTO email_invitation (id,sender,email,space,created_at,expires_at) SELECT ?1,?2,?3,?4,?5,?6 WHERE (SELECT COUNT(*) FROM email_invitation WHERE sender=?2 AND created_at>?5-86400)<20 AND (SELECT COUNT(*) FROM email_invitation WHERE email=?3 AND created_at>?5-86400)<5";
pub(crate) const SELECT: &str = "SELECT * FROM email_invitation WHERE id=?1";
pub(crate) const PUBLISH: &str = "UPDATE email_invitation SET original=?2 WHERE id=?1 AND original IS NULL AND cancelled=0 AND expires_at>?3";
pub(crate) const LEASE: &str = "UPDATE email_invitation SET attempts=attempts+1,next_attempt_at=?2+60 WHERE id=?1 AND original IS NOT NULL AND sent_at IS NULL AND cancelled=0 AND expires_at>?2 AND created_at>?2-82800 AND attempts<3 AND next_attempt_at<=?2";
pub(crate) const SENT: &str =
    "UPDATE email_invitation SET sent_at=COALESCE(sent_at,?2) WHERE id=?1";
pub(crate) const COMPLETE: &str = "UPDATE email_invitation SET recipient=?2,completed=?3 WHERE id=?1 AND original IS NOT NULL AND recipient IS NULL AND cancelled=0 AND expires_at>?4 AND EXISTS (SELECT 1 FROM customer WHERE account=?2 AND email=email_invitation.email AND status='Active')";
pub(crate) const CANCEL: &str = "UPDATE email_invitation SET cancelled=1 WHERE id=?1 AND sender=?2 AND recipient IS NULL AND cancelled=0";
