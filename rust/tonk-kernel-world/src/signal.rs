//! A signal: a count that each [`Signal::notify`] moves, and that a task
//! waits on with [`Signal::changed`] until it moves. It wakes a task that
//! waits only when something happened, so a task that waits on it costs no
//! wakeup while nothing does. See PDD007, "Storage".

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};
use std::task::{Context, Poll, Waker};

/// A signal. Clones share it.
#[derive(Clone, Default)]
pub struct Signal(Arc<Mutex<State>>);

#[derive(Default)]
struct State {
    count: u64,
    /// The tasks that wait for the count to move, by the key of their
    /// [`Changed`]. A wait that drops removes its own, so a signal that is
    /// never notified holds no waker of a wait that is gone.
    wakers: BTreeMap<u64, Waker>,
    /// The key of the next wait.
    next: u64,
}

impl Signal {
    /// A signal that nothing notified yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Moves the count, and wakes each task that waits.
    pub fn notify(&self) {
        let wakers = {
            let mut state = self.state();
            state.count = state.count.wrapping_add(1);
            std::mem::take(&mut state.wakers)
        };
        for waker in wakers.into_values() {
            waker.wake();
        }
    }

    /// Resolves to the count once it is not `since`.
    pub fn changed(&self, since: u64) -> Changed {
        Changed {
            signal: self.clone(),
            since,
            key: None,
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The wait of [`Signal::changed`].
pub struct Changed {
    signal: Signal,
    since: u64,
    /// The key of its waker in the signal, once it waits.
    key: Option<u64>,
}

impl Future for Changed {
    type Output = u64;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<u64> {
        let this = self.get_mut();
        let mut state = this.signal.state();
        if state.count != this.since {
            if let Some(key) = this.key.take() {
                state.wakers.remove(&key);
            }
            return Poll::Ready(state.count);
        }
        // A notify takes each waker, so a wait that was polled before can
        // need a new one.
        let key = *this.key.get_or_insert_with(|| {
            state.next = state.next.wrapping_add(1);
            state.next
        });
        match state.wakers.get_mut(&key) {
            Some(waker) if waker.will_wake(context.waker()) => {}
            Some(waker) => waker.clone_from(context.waker()),
            None => {
                state.wakers.insert(key, context.waker().clone());
            }
        }
        Poll::Pending
    }
}

impl Drop for Changed {
    fn drop(&mut self) {
        if let Some(key) = self.key {
            self.signal.state().wakers.remove(&key);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::future::Future;

    use super::Signal;

    #[tokio::test]
    async fn it_resolves_once_the_count_moves() {
        let signal = Signal::new();
        let since = signal.count();
        let waiting = tokio::spawn(signal.changed(since));
        tokio::task::yield_now().await;
        assert!(!waiting.is_finished(), "nothing notified the signal yet");

        signal.notify();

        assert_eq!(waiting.await.unwrap(), since + 1);
        assert_eq!(
            signal.changed(since).await,
            since + 1,
            "a late wait sees it"
        );
    }

    #[test]
    fn it_holds_no_waker_of_a_wait_that_dropped() {
        let signal = Signal::new();
        let mut context = std::task::Context::from_waker(std::task::Waker::noop());
        for _ in 0..3 {
            let mut waiting = Box::pin(signal.changed(signal.count()));
            assert!(waiting.as_mut().poll(&mut context).is_pending());
            assert!(waiting.as_mut().poll(&mut context).is_pending());
            assert_eq!(signal.state().wakers.len(), 1, "one waker for each wait");
        }
        assert!(
            signal.state().wakers.is_empty(),
            "each wait took its waker when it dropped"
        );
    }
}
