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
use dialog_common::log;

use crate::BranchState;
use crate::env::SelectProvider;
use crate::{Conclusion, Frame, project};

use super::state::QueryHash;

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
    pub async fn perform<'a, Env: SelectProvider>(self, env: &'a Env)
    where
        Self: 'a,
    {
        // Snapshot what we need out of the map lock: the engine
        // handle and the projection terms. Whether a subscriber needs
        // a full snapshot is decided at fan-out time (under the lock),
        // NOT here: a subscriber can attach while the engine poll below
        // awaits, so a flag captured now would miss it and leave it
        // hung on `loading` forever (the stuck-spinner race).
        let (slot, terms) = {
            let subs = self.state.subscriptions().lock();
            let Some(subscription) = subs.get(&self.hash) else {
                return;
            };
            (Arc::clone(&subscription.engine), subscription.terms.clone())
        };

        // Take the engine out of its slot so the poll runs without a
        // guard held across the `await` (see `EngineSlot`). If a
        // concurrent poll holds the engine, wait for it to finish and
        // then run — do NOT bail: a subscriber that just attached (and
        // that the in-flight poll may not have seen when it captured its
        // `any_pending`) still needs its first snapshot, and bailing here
        // left it hung with no data forever ("query gets nothing and
        // gets stuck"). Yielding-and-retrying serves it on the next turn.
        // Bounded so a long-running in-flight poll (its own network await)
        // can't make this spin indefinitely; if still contended after the
        // cap, give up this turn — the in-flight poll's fan-out serves the
        // subscribers it sees, and any genuinely-missed Pending subscriber
        // is re-served by the next scheduled poll.
        const SLOT_RETRY_CAP: u32 = 64;
        let mut engine = 'acquire: {
            let mut attempts = 0;
            loop {
                {
                    let mut guard = slot.lock().await;
                    if let Some(engine) = guard.take() {
                        break 'acquire engine;
                    }
                }
                attempts += 1;
                if attempts >= SLOT_RETRY_CAP {
                    return;
                }
                // Slot busy — yield a microtask so the in-flight poll can
                // put the engine back, then retry acquiring it.
                yield_once().await;
            }
        };

        // The engine gates on both the tree revision and the branch's
        // overlay epoch, so an overlay write (which doesn't move the
        // tree) still re-evaluates on this poll — no re-seed here.
        let poll_result = engine.poll(env).await;

        // Project the engine's retained results while we still hold the
        // engine — the snapshot any Pending subscriber will need. Kept as
        // projected conclusions (not yet serialized) so the fan-out below
        // can decide, under the map lock, whether a snapshot is actually
        // needed. Serializing eagerly here would waste work on the common
        // no-Pending poll; deferring it to fan-out is what lets us serve a
        // subscriber that attached during the poll `await` above without
        // paying for a snapshot when there is none.
        let snapshot_conclusions: Vec<Conclusion> = engine
            .results()
            .iter()
            .map(|c| project(c, &terms))
            .collect();

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
        // reported a NON-EMPTY change. An empty delta (asserted and
        // retracted both empty) is a no-op: a re-evaluation whose
        // result set didn't move (e.g. an overlay epoch bump that
        // changed nothing this query reads). Broadcasting it would
        // repaint every consumer for nothing and, if a re-poll keeps
        // firing, spin a render loop — so drop it here.
        let delta_bytes = delta.as_ref().filter(|d| !d.is_empty()).and_then(|delta| {
            let asserted = delta.asserted.iter().map(|c| project(c, &terms)).collect();
            let retracted = delta.retracted.iter().map(|c| project(c, &terms)).collect();
            serialize(&Frame::Delta {
                asserted,
                retracted,
            })
        });

        // Fan out under the map lock. The delivery decision (snapshot to
        // Pending, delta to Established) lives on `Subscription::deliver`
        // so it re-checks the subscriber set HERE, under the lock, rather
        // than off a flag captured before the poll `await` — a subscriber
        // that attached during the poll is served its first snapshot now
        // instead of hanging until some later write schedules another poll.
        let mut subs = self.state.subscriptions().lock();
        let Some(subscription) = subs.get_mut(&self.hash) else {
            return;
        };
        subscription.deliver(&snapshot_conclusions, delta_bytes.as_ref());
        if subscription.subscribers.is_empty() {
            subs.remove(&self.hash);
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

/// Yield control once: return `Pending` a single time (waking immediately),
/// so the executor can advance another task before this one resumes.
/// Runtime-agnostic (works on the wasm service worker and native), used to
/// let an in-flight poll release the engine slot before this poll retries.
async fn yield_once() {
    use std::future::Future;
    use std::pin::Pin;
    use std::task::{Context, Poll};

    struct YieldOnce(bool);
    impl Future for YieldOnce {
        type Output = ();
        fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
            if self.0 {
                Poll::Ready(())
            } else {
                self.0 = true;
                cx.waker().wake_by_ref();
                Poll::Pending
            }
        }
    }
    YieldOnce(false).await
}
