//! Subscription chain handles.
//!
//! `branch_session.subscription(hash)` returns a
//! [`SubscriptionReference`]; chain `.poll()` for a
//! [`SubscriptionPoll`] effect, then `.perform(&env)` to
//! re-evaluate and broadcast.
//!
//! Pure descriptions: both types just hold an `&Arc<BranchState>`
//! and the subscription hash. Nothing locks until `perform`.

use std::sync::Arc;

use bytes::Bytes;
use dialog_common::Blake3Hash;
use dialog_query::Output as _;
use tonk_common::log;

use crate::reactor::BranchState;
use crate::reactor::conclusion::Conclusion;
use crate::reactor::env::SelectProvider;

use super::state::{QueryHash, Status};

/// Names a subscription within a branch. Built from
/// [`BranchSession::subscription`].
///
/// [`BranchSession::subscription`]: crate::reactor::BranchSession::subscription
#[derive(Clone)]
pub struct SubscriptionReference<'a> {
    /// The branch state that owns the subscription map.
    pub state: &'a Arc<BranchState>,
    /// Identity of this subscription within the branch.
    pub hash: QueryHash,
}

impl<'a> SubscriptionReference<'a> {
    /// Build a [`SubscriptionPoll`] effect — `.perform(&env)` to
    /// run the poll.
    pub fn poll(self) -> SubscriptionPoll<'a> {
        SubscriptionPoll {
            state: self.state,
            hash: self.hash,
        }
    }
}

/// Effect — re-evaluate the named subscription and broadcast.
pub struct SubscriptionPoll<'a> {
    /// The branch state that owns the subscription map.
    pub state: &'a Arc<BranchState>,
    /// Identity of the subscription to poll.
    pub hash: QueryHash,
}

impl SubscriptionPoll<'_> {
    /// Re-evaluate the subscription, decide who receives the
    /// bytes (Pending vs Established), update `last_hash`,
    /// drop dead subscribers, drop the subscription if empty.
    pub async fn perform<Env: SelectProvider>(self, env: &Env) {
        // Snapshot the query out of the lock so we can run it
        // without holding the subscription mutex across await.
        let query = {
            let subs = self.state.subscriptions().lock();
            let Some(subscription) = subs.get(&self.hash) else {
                return;
            };
            subscription.query.clone()
        };

        let conclusions = match self
            .state
            .branch
            .query()
            .select(query)
            .perform(env)
            .try_vec()
            .await
        {
            Ok(c) => c,
            Err(err) => {
                log!("[reactor] subscription poll failed: {err:?}");
                return;
            }
        };

        let wire: Vec<Conclusion> = conclusions.iter().map(Conclusion::from).collect();
        let bytes = match serde_json::to_vec(&wire) {
            Ok(b) => Bytes::from(b),
            Err(err) => {
                log!("[reactor] failed to serialize conclusions: {err}");
                return;
            }
        };
        let new_hash = Blake3Hash::hash(&bytes);

        let mut subs = self.state.subscriptions().lock();
        let Some(subscription) = subs.get_mut(&self.hash) else {
            return;
        };

        let changed = subscription.last_hash.as_ref() != Some(&new_hash);
        if changed {
            subscription.last_hash = Some(new_hash);
        }

        // Walk subscribers, sending where required, dropping any
        // whose receiver closed.
        subscription.subscribers.retain_mut(|subscriber| {
            let needs_send = changed || subscriber.status == Status::Pending;
            if !needs_send {
                return true;
            }
            match subscriber.sender.send(bytes.clone()) {
                Ok(()) => {
                    subscriber.status = Status::Established;
                    true
                }
                Err(_) => false,
            }
        });

        if subscription.subscribers.is_empty() {
            subs.remove(&self.hash);
        }
    }
}
