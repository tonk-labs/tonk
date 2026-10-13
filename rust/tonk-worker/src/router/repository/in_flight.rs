//! Template copies this worker is running right now.
//!
//! A copy link (`/copy/<slug>`) starts a copy as soon as the page lands, so a
//! reload, a second tab or a re-rendered dialog sends the same
//! `space/create` again while the first is still fetching. Each repeat joins
//! the copy already running as a waiter instead of making a second space,
//! and every waiter is answered — receipt and navigation — when it reports.
//!
//! The set lives only in this worker's memory, like
//! [`SeedUpgrades`](crate::router::adopt::SeedUpgrades): a worker the browser
//! stops forgets it together with the copy it was running, so nothing can be
//! left "in flight" forever. A [`Claim`] clears its entry on every exit, and
//! [`Claim::release`] clears it at the moment the copy reports, before the
//! slow remote attach that follows.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use super::OpenMode;

/// One request waiting on a copy: where to write its result and how to
/// move the page that sent it.
#[derive(Clone, Debug)]
pub(crate) struct Waiter {
    pub(crate) receipt: dialog_artifacts::Entity,
    pub(crate) client: Option<crate::router::ClientId>,
    pub(crate) open: OpenMode,
}

/// The copies in flight, keyed by [`key`].
#[derive(Clone, Default)]
pub(crate) struct CopiesInFlight(Arc<Mutex<HashMap<String, Vec<Waiter>>>>);

/// The identity of a copy: the template reference plus the name the request
/// asked for (before the worker picks the final label). A reload sends both
/// unchanged; a Discover copy with a name the person typed is a different
/// copy and runs on its own.
pub(crate) fn key(template: &str, requested_name: &str) -> String {
    format!("{}\u{0}{}", template.trim(), requested_name.trim())
}

impl CopiesInFlight {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Vec<Waiter>>> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Start the copy at `key`, or join the one already running.
    ///
    /// `Some(claim)` means this request runs the copy and must answer every
    /// waiter through [`Claim::release`]. `None` means it joined a running
    /// copy, which will answer it.
    pub(crate) fn claim(&self, key: &str, waiter: Waiter) -> Option<Claim> {
        let mut copies = self.lock();
        if let Some(waiters) = copies.get_mut(key) {
            waiters.push(waiter);
            return None;
        }
        copies.insert(key.to_owned(), vec![waiter]);
        Some(Claim {
            copies: self.clone(),
            key: Some(key.to_owned()),
        })
    }

    #[cfg(test)]
    fn contains(&self, key: &str) -> bool {
        self.lock().contains_key(key)
    }
}

/// The right to run one copy. Dropping it without [`release`](Self::release)
/// (an early return, a panic) still clears the entry, so a later request
/// starts a fresh copy rather than waiting on one that will never report.
pub(crate) struct Claim {
    copies: CopiesInFlight,
    key: Option<String>,
}

impl Claim {
    /// Clear the entry and hand back every waiter, the claimant first.
    /// A request that arrives after this starts a new copy.
    pub(crate) fn release(mut self) -> Vec<Waiter> {
        self.key
            .take()
            .and_then(|key| self.copies.lock().remove(&key))
            .unwrap_or_default()
    }
}

impl Drop for Claim {
    fn drop(&mut self) {
        if let Some(key) = self.key.take() {
            self.copies.lock().remove(&key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_service_worker);

    fn waiter(receipt: &str, open: OpenMode) -> Waiter {
        Waiter {
            receipt: receipt.parse().unwrap(),
            client: Some(crate::router::ClientId(receipt.to_owned())),
            open,
        }
    }

    #[dialog_common::test]
    fn a_repeat_joins_the_running_copy_and_both_are_answered() {
        let copies = CopiesInFlight::default();
        let key = key("https://catalog.test/c.json#demo", "Untitled");
        let claim = copies
            .claim(&key, waiter("urn:uuid:first", OpenMode::Replace))
            .expect("the first request runs the copy");
        assert!(
            copies
                .claim(&key, waiter("urn:uuid:reload", OpenMode::Replace))
                .is_none(),
            "a reload joins instead of starting a second copy"
        );
        let waiters = claim.release();
        let receipts: Vec<String> = waiters.iter().map(|w| w.receipt.to_string()).collect();
        assert_eq!(receipts, ["urn:uuid:first", "urn:uuid:reload"]);
        assert!(!copies.contains(&key), "released at report");
    }

    #[dialog_common::test]
    fn after_release_a_new_request_starts_a_new_copy() {
        let copies = CopiesInFlight::default();
        let key = key("https://catalog.test/c.json#demo", "Untitled");
        copies
            .claim(&key, waiter("urn:uuid:a", OpenMode::Navigate))
            .unwrap()
            .release();
        assert!(
            copies
                .claim(&key, waiter("urn:uuid:b", OpenMode::Navigate))
                .is_some()
        );
    }

    #[dialog_common::test]
    fn dropping_a_claim_clears_it() {
        let copies = CopiesInFlight::default();
        let key = key("https://catalog.test/c.json#demo", "Untitled");
        drop(copies.claim(&key, waiter("urn:uuid:a", OpenMode::Navigate)));
        assert!(
            !copies.contains(&key),
            "an early return must not leave the copy in flight"
        );
    }

    #[dialog_common::test]
    fn a_typed_name_is_a_different_copy() {
        let copies = CopiesInFlight::default();
        let template = "https://catalog.test/c.json#demo";
        let _running = copies
            .claim(
                &key(template, "Untitled"),
                waiter("urn:uuid:link", OpenMode::Replace),
            )
            .unwrap();
        assert!(
            copies
                .claim(
                    &key(template, "My recipes"),
                    waiter("urn:uuid:typed", OpenMode::Navigate)
                )
                .is_some(),
            "a Discover copy with its own name runs on its own"
        );
    }
}
