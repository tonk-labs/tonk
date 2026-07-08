//! Subscription chain handles.
//!
//! `branch_session.subscription(hash)` returns a
//! [`SubscriptionReference`]; chain `.poll()` for a
//! [`SubscriptionPoll`] effect, then `.perform(&env)` to
//! re-evaluate and broadcast.
//!
//! Pure descriptions: both types just hold an `&Arc<BranchState>`
//! and the subscription hash. Nothing locks until `perform`.

use std::future::Future;
use std::sync::Arc;

use bytes::Bytes;
use dialog_common::log;

use crate::BranchState;
use crate::env::SelectProvider;
use crate::{Conclusion, Frame};

use super::state::{QueryHash, Status};

/// Names a subscription within a branch. Built from
/// [`BranchSession::subscription`].
///
/// [`BranchSession::subscription`]: crate::BranchSession::subscription
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
    /// Poll the subscription's dialog engine, then broadcast.
    ///
    /// The engine ([`dialog_repository::Subscription`]) is
    /// demand-gated: its [`poll`](dialog_repository::Subscription::poll)
    /// returns `None` when nothing inside the query's demand cover
    /// changed — no query runs, nothing broadcasts (the win). A
    /// change yields a [`Delta`](dialog_repository::Delta) of
    /// asserted / retracted rows.
    ///
    /// Two delivery cases per subscriber:
    /// - **Pending** (just attached, or reconnected): gets a
    ///   [`Frame::Snapshot`] built from the engine's retained
    ///   `results()`, so it needs no prior state. Delivered even when
    ///   the poll itself found no change.
    /// - **Established**: gets a [`Frame::Delta`] whenever the poll
    ///   reports one.
    ///
    /// Locking dance: the engine's `poll` takes `&mut self` and
    /// awaits, so it can't run under the branch's synchronous
    /// subscription-map lock. We clone the `Arc<Engine>` (and the
    /// projection terms) out under the map lock, await the engine
    /// under its own async lock, then re-lock the map only to fan
    /// bytes out.
    pub fn perform<'a, Env: SelectProvider>(self, env: &'a Env) -> impl Future<Output = ()> + 'a
    where
        Self: 'a,
    {
        // Returned as `impl Future + 'a` (not `async fn`) so the env
        // lifetime is named rather than higher-ranked — the same
        // reason dialog's `Subscription::poll` is written this way.
        // An `async fn perform<Env>(self, env: &Env)` desugars to a
        // future whose `&Env` capture is `for<'e>`, which defeats
        // Send-generality inference in the axum handlers that drain
        // scheduled polls, making every one of them `!Send`.
        async move {
            // Snapshot what we need out of the map lock: the engine
            // handle, the projection terms, the subscription's seeded
            // overlay epoch, and whether any subscriber is Pending (needs
            // a full snapshot rather than a delta).
            let (slot, terms, seeded_epoch, any_pending) = {
                let subs = self.state.subscriptions().lock();
                let Some(subscription) = subs.get(&self.hash) else {
                    return;
                };
                let any_pending = subscription
                    .subscribers
                    .iter()
                    .any(|s| s.status == Status::Pending);
                (
                    Arc::clone(&subscription.engine),
                    subscription.terms.clone(),
                    subscription.seeded_overlay_epoch,
                    any_pending,
                )
            };

            // Re-seed the engine's overlay if the branch's overlay moved
            // since this subscription was seeded. An overlay write is
            // off-tree, so the engine's tree-diff gate can't see it;
            // `set_overlay` invalidates the engine's pin so the poll below
            // recomputes and reports the overlay-induced change as a delta.
            let current_epoch = self.state.overlay_epoch();

            // Take the engine out of its slot so the poll runs without a
            // guard held across the `await` (see `EngineSlot`). A
            // concurrent poll of the same subscription finds the slot
            // emptied and returns — its in-flight sibling covers the same
            // work.
            let mut engine = {
                let mut guard = slot.lock().await;
                let Some(engine) = guard.take() else {
                    return;
                };
                engine
            };

            if seeded_epoch != current_epoch {
                engine.set_overlay(self.state.overlay());
            }

            let poll_result = engine.poll(env).await;

            // Snapshot for Pending subscribers — projected from the
            // engine's retained results, consistent with the poll we just
            // ran. Built only when some subscriber needs it.
            let snapshot_bytes = if any_pending {
                let conclusions: Vec<Conclusion> = engine
                    .results()
                    .iter()
                    .map(|c| Conclusion::project(c, &terms))
                    .collect();
                serialize(&Frame::Snapshot { conclusions })
            } else {
                None
            };

            // Put the engine back before handling the poll outcome so the
            // slot is never left empty on an early return.
            *slot.lock().await = Some(engine);

            let delta = match poll_result {
                Ok(delta) => delta,
                Err(err) => {
                    log!("[reactor] subscription poll failed: {err:?}");
                    return;
                }
            };

            // Delta for Established subscribers — only when the poll
            // reported a change.
            let delta_bytes = delta.as_ref().and_then(|delta| {
                let asserted = delta
                    .asserted
                    .iter()
                    .map(|c| Conclusion::project(c, &terms))
                    .collect();
                let retracted = delta
                    .retracted
                    .iter()
                    .map(|c| Conclusion::project(c, &terms))
                    .collect();
                serialize(&Frame::Delta {
                    asserted,
                    retracted,
                })
            });

            // Fan out under the map lock.
            let mut subs = self.state.subscriptions().lock();
            let Some(subscription) = subs.get_mut(&self.hash) else {
                return;
            };
            subscription.seeded_overlay_epoch = current_epoch;

            subscription.subscribers.retain_mut(|subscriber| {
                let bytes = match subscriber.status {
                    // A fresh (or reconnected) subscriber needs the full
                    // set, not a delta it has no base for.
                    Status::Pending => snapshot_bytes.clone(),
                    // An established subscriber only hears about changes.
                    Status::Established => delta_bytes.clone(),
                };
                let Some(bytes) = bytes else {
                    // Nothing to send this subscriber this round (an
                    // Established subscriber on a no-change poll, or a
                    // frame that failed to serialize) — keep it.
                    return true;
                };
                match subscriber.sender.send(bytes) {
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
}

/// Serialize a [`Frame`] to wire bytes, logging and dropping on
/// failure (a serialization error is not worth killing the poll).
fn serialize(frame: &Frame) -> Option<Bytes> {
    match serde_json::to_vec(frame) {
        Ok(bytes) => Some(Bytes::from(bytes)),
        Err(err) => {
            log!("[reactor] failed to serialize frame: {err}");
            None
        }
    }
}
