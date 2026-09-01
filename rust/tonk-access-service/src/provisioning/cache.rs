//! Cached servability verdicts, so the presign hot path does not read
//! control D1 per request.
//!
//! plan/Access metering.md §7 and §11 specify the resolution order this
//! implements: isolate cache, then KV, then control D1 on a miss, with
//! the derived verdict written back. The gate's authority stays in
//! [`screen`](crate::provisioning::screen); this module only remembers
//! its answers for a bounded time.
//!
//! Staleness is asymmetric, per the plan: a stale permit costs a
//! handful of operations, a stale denial costs a paying customer
//! service. So a serving verdict lives [`OK_VALIDITY`] and a denial
//! only [`DENY_VALIDITY`], and validity carries jitter so expiry does
//! not cluster into a synchronised read storm against D1.
//!
//! `not_after` is an absolute timestamp inside the value rather than a
//! store TTL: the isolate copy ages after it is read, and KV's own
//! expiry (minimum 60 seconds) is only garbage collection.

use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};

use dialog_capability::access::AuthorizeError;
use serde::{Deserialize, Serialize};

/// The KV namespace binding holding cached verdicts.
pub const BINDING: &str = "SERVABILITY_KV";

/// The value shape this build writes and accepts. A stale isolate can
/// hold a shape written by a previous deploy, so a value declaring any
/// other version reads as a miss rather than as garbage.
const VERSION: u32 = 1;

/// How long a serving verdict stands before revalidation, in seconds.
const OK_VALIDITY: u64 = 300;

/// How long a denial stands, in seconds. Short, because the states it
/// caches clear by someone acting — provisioning, activation — and the
/// person who acted is usually watching the retry.
const DENY_VALIDITY: u64 = 30;

/// Entries kept in the isolate cache before it is pruned.
const ISOLATE_CAPACITY: usize = 1024;

/// A remembered gate verdict: serving when `deny` is absent, refused
/// with exactly the recorded error otherwise.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CachedVerdict {
    /// Value-shape version; see [`VERSION`].
    pub v: u32,
    /// Absolute expiry: a reader at or past this moment revalidates.
    pub not_after: u64,
    /// The refusal, when the verdict is a refusal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deny: Option<AuthorizeError>,
}

impl CachedVerdict {
    /// Record `outcome` as a verdict valid from `now`, with per-variant
    /// validity and jitter keyed off the subject.
    pub fn record(outcome: &Result<(), AuthorizeError>, subject: &str, now: u64) -> Self {
        let base = match outcome {
            Ok(()) => OK_VALIDITY,
            Err(_) => DENY_VALIDITY,
        };
        Self {
            v: VERSION,
            not_after: now + base + jitter(subject, now, base / 5),
            deny: outcome.as_ref().err().cloned(),
        }
    }

    /// Whether this verdict may still be used at `now`.
    pub fn fresh(&self, now: u64) -> bool {
        now < self.not_after
    }

    /// The gate outcome this verdict remembers.
    pub fn verdict(&self) -> Result<(), AuthorizeError> {
        match &self.deny {
            None => Ok(()),
            Some(deny) => Err(deny.clone()),
        }
    }

    /// The verdict as its stored JSON.
    pub fn encode(&self) -> String {
        serde_json::to_string(self).expect("verdict serializes")
    }

    /// Read a stored value back; `None` — a miss — for anything this
    /// build does not recognize.
    pub fn decode(text: &str) -> Option<Self> {
        serde_json::from_str::<Self>(text)
            .ok()
            .filter(|cached| cached.v == VERSION)
    }

    /// Seconds the backing store should retain this value. At least
    /// KV's 60-second minimum; `not_after` inside the value is what
    /// governs validity.
    pub fn retention(&self, now: u64) -> u64 {
        (self.not_after.saturating_sub(now)).max(60)
    }
}

/// The KV key for a subject's verdict.
pub fn key(subject: &str) -> String {
    format!("servability/{subject}")
}

/// Deterministic spread in `0..=range`, so verdicts recorded together
/// do not all expire together.
fn jitter(subject: &str, now: u64, range: u64) -> u64 {
    let mut hasher = DefaultHasher::new();
    subject.hash(&mut hasher);
    now.hash(&mut hasher);
    match range {
        0 => 0,
        range => hasher.finish() % (range + 1),
    }
}

thread_local! {
    /// This isolate's own verdicts. Workers isolates are single
    /// threaded, so thread local is isolate local.
    static ISOLATE: RefCell<HashMap<String, CachedVerdict>> = RefCell::new(HashMap::new());
}

/// The isolate's fresh verdict for `subject`, if it holds one.
pub fn isolate_lookup(subject: &str, now: u64) -> Option<CachedVerdict> {
    ISOLATE.with(|cell| {
        cell.borrow()
            .get(subject)
            .filter(|cached| cached.fresh(now))
            .cloned()
    })
}

/// Remember `verdict` in this isolate. At capacity the expired entries
/// are pruned first; a cache still full of fresh entries is cleared —
/// correctness never depends on retention.
pub fn isolate_store(subject: &str, verdict: CachedVerdict, now: u64) {
    ISOLATE.with(|cell| {
        let mut entries = cell.borrow_mut();
        if entries.len() >= ISOLATE_CAPACITY {
            entries.retain(|_, cached| cached.fresh(now));
        }
        if entries.len() >= ISOLATE_CAPACITY {
            entries.clear();
        }
        entries.insert(subject.to_string(), verdict);
    });
}

/// Drop this isolate's verdict for `subject`, so a state change made
/// here is visible here immediately. Other isolates converge by
/// `not_after`.
pub fn isolate_forget(subject: &str) {
    ISOLATE.with(|cell| {
        cell.borrow_mut().remove(subject);
    });
}

#[cfg(test)]
mod tests {
    use dialog_capability::access::Recourse;

    use super::*;

    fn denial() -> AuthorizeError {
        AuthorizeError::Declined {
            recourse: Recourse::Retry,
            reason: "the subject's own registration awaits email activation".into(),
        }
    }

    #[test]
    fn it_round_trips_a_serving_verdict() {
        let recorded = CachedVerdict::record(&Ok(()), "did:key:zAlice", 1_000);
        let decoded = CachedVerdict::decode(&recorded.encode()).expect("decodes");
        assert_eq!(decoded, recorded);
        assert!(decoded.verdict().is_ok());
    }

    #[test]
    fn it_round_trips_a_denial_with_its_recourse() {
        let recorded = CachedVerdict::record(&Err(denial()), "did:key:zAlice", 1_000);
        let decoded = CachedVerdict::decode(&recorded.encode()).expect("decodes");
        assert_eq!(decoded.verdict(), Err(denial()));
    }

    #[test]
    fn it_gives_denials_shorter_validity_than_permits() {
        let ok = CachedVerdict::record(&Ok(()), "did:key:zAlice", 1_000);
        let deny = CachedVerdict::record(&Err(denial()), "did:key:zAlice", 1_000);
        assert!(deny.not_after < ok.not_after);
        assert!(deny.fresh(1_000 + DENY_VALIDITY - 1));
        assert!(!deny.fresh(1_000 + DENY_VALIDITY + DENY_VALIDITY / 5 + 1));
        assert!(!ok.fresh(1_000 + OK_VALIDITY + OK_VALIDITY / 5 + 1));
    }

    #[test]
    fn it_treats_an_unknown_version_as_a_miss() {
        let mut recorded = CachedVerdict::record(&Ok(()), "did:key:zAlice", 1_000);
        recorded.v = VERSION + 1;
        assert_eq!(CachedVerdict::decode(&recorded.encode()), None);
        assert_eq!(CachedVerdict::decode("not json"), None);
    }

    #[test]
    fn it_retains_at_least_the_kv_minimum() {
        let deny = CachedVerdict::record(&Err(denial()), "did:key:zAlice", 1_000);
        assert!(deny.retention(1_000) >= 60);
        let ok = CachedVerdict::record(&Ok(()), "did:key:zAlice", 1_000);
        assert!(ok.retention(1_000) >= OK_VALIDITY);
    }

    #[test]
    fn it_serves_from_the_isolate_until_expiry_and_forgets_on_demand() {
        let recorded = CachedVerdict::record(&Ok(()), "did:key:zIsolate", 1_000);
        isolate_store("did:key:zIsolate", recorded.clone(), 1_000);
        assert_eq!(isolate_lookup("did:key:zIsolate", 1_000), Some(recorded));
        assert_eq!(isolate_lookup("did:key:zIsolate", u64::MAX), None);
        isolate_forget("did:key:zIsolate");
        assert_eq!(isolate_lookup("did:key:zIsolate", 1_000), None);
    }
}
