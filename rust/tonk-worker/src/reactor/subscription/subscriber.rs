//! [`Subscriber`] — handle returned to a caller that just
//! attached to a subscription.
//!
//! Carries the subscription's hash (so the caller can poll it
//! by name via `branch_session.subscription(hash).poll()`) plus
//! the receiver they read from.

use bytes::Bytes;
use tokio::sync::mpsc::UnboundedReceiver;

use super::QueryHash;

/// Handle returned by `BranchState::subscribe`. Holds the
/// subscription's hash and the receiver to read broadcast bytes
/// from.
pub struct Subscriber {
    /// The subscription's identity within its branch. Used to
    /// poll the subscription via
    /// `branch_session.subscription(hash).poll().perform(&env)`.
    pub hash: QueryHash,
    /// Receiver side of the mpsc channel — yields one `Bytes`
    /// per broadcast.
    pub receiver: UnboundedReceiver<Bytes>,
}
