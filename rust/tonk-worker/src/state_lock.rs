//! The worker's state lock, metered.
//!
//! [`StateLock`] is the `tokio::sync::RwLock` every route takes to reach
//! [`TonkState`](crate::worker::TonkState), with the same methods, plus a
//! record in [`tonk_common::diagnostics`] of how long each acquisition waited
//! and how long each guard was held, keyed by the source line that took it.
//!
//! The lock is write-preferring, so a guard held across a network round trip
//! stalls every request queued behind it. The hold times by site are how to
//! find which holds those are; this module measures them and changes nothing
//! about who waits for whom.

use std::ops::{Deref, DerefMut};
use std::panic::Location;
use std::sync::Arc;

use tokio::sync::{OwnedRwLockReadGuard, RwLock, RwLockReadGuard, RwLockWriteGuard, TryLockError};
use tonk_common::diagnostics::{self, names};

/// A read-write lock that records waits and holds. See the module docs.
#[derive(Debug, Default)]
pub struct StateLock<T> {
    lock: Arc<RwLock<T>>,
}

impl<T> StateLock<T> {
    /// Wrap `value` in a new, unlocked lock.
    pub fn new(value: T) -> Self {
        Self {
            lock: Arc::new(RwLock::new(value)),
        }
    }

    /// Consume the lock and return the value it guards.
    ///
    /// # Panics
    ///
    /// If an owned read guard taken from this lock is still alive.
    pub fn into_inner(self) -> T {
        match Arc::try_unwrap(self.lock) {
            Ok(lock) => lock.into_inner(),
            Err(_) => panic!("StateLock::into_inner while an owned read guard is alive"),
        }
    }

    /// Take a read guard, waiting while a writer holds or is queued for the
    /// lock.
    #[track_caller]
    pub fn read(&self) -> impl Future<Output = ReadGuard<'_, T>> {
        let site = Location::caller();
        async move {
            let guard = match self.lock.try_read() {
                Ok(guard) => guard,
                Err(_) => {
                    let start = diagnostics::now_ms();
                    let guard = self.lock.read().await;
                    diagnostics::record_at(
                        names::WORKER_LOCK_READ_WAIT,
                        site,
                        diagnostics::now_ms() - start,
                    );
                    guard
                }
            };
            ReadGuard {
                guard,
                _held: Held::start(names::WORKER_LOCK_READ_HOLD, site),
            }
        }
    }

    /// Take the write guard, waiting for every other guard to be released.
    #[track_caller]
    pub fn write(&self) -> impl Future<Output = WriteGuard<'_, T>> {
        let site = Location::caller();
        async move {
            let guard = match self.lock.try_write() {
                Ok(guard) => guard,
                Err(_) => {
                    let start = diagnostics::now_ms();
                    let guard = self.lock.write().await;
                    diagnostics::record_at(
                        names::WORKER_LOCK_WRITE_WAIT,
                        site,
                        diagnostics::now_ms() - start,
                    );
                    guard
                }
            };
            WriteGuard {
                guard,
                _held: Held::start(names::WORKER_LOCK_WRITE_HOLD, site),
            }
        }
    }

    /// Take a read guard that owns its reference to the lock, so it can
    /// outlive the borrow it was taken from.
    #[track_caller]
    pub fn read_owned(self: Arc<Self>) -> impl Future<Output = OwnedReadGuard<T>>
    where
        T: 'static,
    {
        let site = Location::caller();
        async move {
            let lock = self.lock.clone();
            let guard = match lock.clone().try_read_owned() {
                Ok(guard) => guard,
                Err(_) => {
                    let start = diagnostics::now_ms();
                    let guard = lock.read_owned().await;
                    diagnostics::record_at(
                        names::WORKER_LOCK_READ_WAIT,
                        site,
                        diagnostics::now_ms() - start,
                    );
                    guard
                }
            };
            OwnedReadGuard {
                guard,
                _held: Held::start(names::WORKER_LOCK_READ_HOLD, site),
            }
        }
    }

    /// Take a read guard only if one is available now.
    #[track_caller]
    pub fn try_read(&self) -> Result<ReadGuard<'_, T>, TryLockError> {
        let site = Location::caller();
        self.lock.try_read().map(|guard| ReadGuard {
            guard,
            _held: Held::start(names::WORKER_LOCK_READ_HOLD, site),
        })
    }

    /// Take the write guard only if it is available now.
    #[track_caller]
    pub fn try_write(&self) -> Result<WriteGuard<'_, T>, TryLockError> {
        let site = Location::caller();
        self.lock.try_write().map(|guard| WriteGuard {
            guard,
            _held: Held::start(names::WORKER_LOCK_WRITE_HOLD, site),
        })
    }
}

/// Records, when dropped, how long the guard it sits in was held.
#[derive(Debug)]
struct Held {
    name: &'static str,
    site: &'static Location<'static>,
    since: f64,
}

impl Held {
    fn start(name: &'static str, site: &'static Location<'static>) -> Self {
        Self {
            name,
            site,
            since: diagnostics::now_ms(),
        }
    }
}

impl Drop for Held {
    fn drop(&mut self) {
        diagnostics::record_at(self.name, self.site, diagnostics::now_ms() - self.since);
    }
}

/// A read guard on a [`StateLock`].
#[derive(Debug)]
pub struct ReadGuard<'a, T> {
    guard: RwLockReadGuard<'a, T>,
    _held: Held,
}

impl<T> Deref for ReadGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.guard
    }
}

/// The write guard on a [`StateLock`].
#[derive(Debug)]
pub struct WriteGuard<'a, T> {
    guard: RwLockWriteGuard<'a, T>,
    _held: Held,
}

impl<T> Deref for WriteGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.guard
    }
}

impl<T> DerefMut for WriteGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.guard
    }
}

/// A read guard on a [`StateLock`] that owns its reference to the lock.
#[derive(Debug)]
pub struct OwnedReadGuard<T> {
    guard: OwnedRwLockReadGuard<T>,
    _held: Held,
}

impl<T> Deref for OwnedReadGuard<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.guard
    }
}

/// Serializes the tests that reset the process-wide diagnostics registry,
/// so one test's reset never clears what another is about to assert.
#[cfg(test)]
pub(crate) static DIAGNOSTICS_TESTS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[cfg(test)]
mod tests {
    use super::*;

    /// How many times `name` was recorded from a line of this file. Other
    /// tests take other locks concurrently; this counts only what these did.
    fn here(name: &str) -> u64 {
        diagnostics::snapshot()
            .sites
            .iter()
            .filter(|(key, _)| {
                key.starts_with(&format!("{name}@")) && key.contains("state_lock.rs")
            })
            .map(|(_, duration)| duration.count)
            .sum()
    }

    #[dialog_common::test]
    async fn it_records_a_hold_for_every_guard_by_the_line_that_took_it() {
        let _serial = DIAGNOSTICS_TESTS.lock().await;
        diagnostics::reset();
        let lock = Arc::new(StateLock::new(1_u32));
        {
            let read = lock.read().await;
            assert_eq!(*read, 1);
        }
        {
            let mut write = lock.write().await;
            *write = 2;
        }
        {
            let owned = lock.clone().read_owned().await;
            assert_eq!(*owned, 2);
        }
        assert!(lock.try_write().is_ok());

        assert_eq!(here(names::WORKER_LOCK_READ_HOLD), 2);
        assert_eq!(here(names::WORKER_LOCK_WRITE_HOLD), 2);
        assert_eq!(here(names::WORKER_LOCK_READ_WAIT), 0);
        assert_eq!(here(names::WORKER_LOCK_WRITE_WAIT), 0);
    }

    #[dialog_common::test]
    async fn it_records_a_wait_when_a_reader_queues_behind_the_writer() {
        let _serial = DIAGNOSTICS_TESTS.lock().await;
        diagnostics::reset();
        let lock = Arc::new(StateLock::new(0_u32));
        let write = lock.write().await;
        assert!(lock.try_read().is_err());
        let reader = {
            let lock = lock.clone();
            async move { *lock.read().await }
        };
        let release = async move {
            drop(write);
        };
        let (value, ()) = futures_util::join!(reader, release);
        assert_eq!(value, 0);
        assert_eq!(here(names::WORKER_LOCK_READ_WAIT), 1);
    }
}
