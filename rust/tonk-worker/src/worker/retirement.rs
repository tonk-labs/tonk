//! The one-way retirement of a worker generation, reachable without the
//! state lock.
//!
//! A verified successor waits for the outgoing worker to release its
//! streams. That release used to take a read guard on the shared state to
//! reach the reactor, so a writer queued behind a slow request held the
//! release, and with it the successor's activation and every request the
//! service worker held for the release. [`Retirement`] keeps a weak handle
//! to the state that is serving, so the release never waits on the lock.

use std::sync::{
    Arc, Weak,
    atomic::{AtomicBool, Ordering},
};

use dialog_reactor::OverlaySnapshot;
use parking_lot::Mutex;

use super::{TonkShared, TonkState};

/// Whether this worker generation is retiring, and the state it serves.
///
/// Shared by every state a profile swap installs, so the latch and the
/// handle outlive any one of them.
#[derive(Default)]
pub struct Retirement {
    latch: AtomicBool,
    /// The state currently installed under the lock. Weak, because the
    /// state owns this through its shared part.
    serving: Mutex<Weak<TonkShared>>,
}

impl Retirement {
    /// Whether retirement has begun. It never ends.
    pub fn is_retiring(&self) -> bool {
        self.latch.load(Ordering::Acquire)
    }

    /// Raise the latch, so no new stream opens on this generation.
    pub fn begin(&self) {
        self.latch.store(true, Ordering::Release);
    }

    /// Record `state` as the one installed under the lock. If retirement
    /// has already begun, its streams are released now: a state installed
    /// after the release would otherwise serve them for the rest of the
    /// generation.
    pub fn serve(&self, state: &TonkState) {
        let mut serving = self.serving.lock();
        *serving = Arc::downgrade(&state.shared);
        if self.is_retiring() {
            state.reactor.shutdown();
        }
    }

    /// Raise the latch and release every stream of the serving state,
    /// returning its session overlays for the successor. Takes no state
    /// lock, so it finishes however long a request holds the state.
    pub fn release(&self) -> Vec<OverlaySnapshot> {
        let serving = self.serving.lock();
        self.begin();
        match serving.upgrade() {
            Some(shared) => {
                let overlays = shared.reactor.export_overlays();
                shared.reactor.shutdown();
                overlays
            }
            None => Vec::new(),
        }
    }
}
