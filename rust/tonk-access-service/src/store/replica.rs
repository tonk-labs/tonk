//! KV replica of the customer row, so the registration probe and the
//! email lookup answer without reading control D1 per request.
//!
//! Both endpoints answer from the same fact — the `customer` row — read
//! by account DID (`GET /customer/:did`) or by address
//! (`GET /customer/:domain/:local/did.json`), and both are polled: an
//! enrolling client watches the probe for activation, an invite flow
//! watches the lookup for the address it just invited. Neither answer
//! is HTTP-cacheable while it is the one being waited on, so before
//! this replica every poll was a D1 read.
//!
//! The row is replicated under both access paths at the same points
//! that commit it to D1 — enrollment and activation write it through,
//! customer deletion drops the DID key — and the read path backfills on
//! a miss. The record rarely changes once activated, so an `Active` row
//! carries generous validity; a `Registered` row is exactly what a
//! poller is waiting to see change, and an absent row is what an invite
//! flow polls through, so both stay short. `not_after` is absolute and
//! inside the value, like the servability verdicts': KV's own expiry
//! (minimum 60 seconds) is only garbage collection.

use std::hash::{DefaultHasher, Hash, Hasher};

use serde::{Deserialize, Serialize};
use tonk_account::customer::CustomerStatus;

use crate::email::normalize_email;
use crate::store::Customer;

/// The value shape this build writes and accepts; any other version
/// reads as a miss.
const VERSION: u32 = 1;

/// How long an `Active` (or `Suspended`) row stands, in seconds. These
/// change through commands that rewrite the replica, so validity only
/// bounds how long a write this service never saw — a manual D1 edit, a
/// missed write-through — goes unnoticed.
const SETTLED_VALIDITY: u64 = 300;

/// How long a `Registered` row or a known-absent answer stands, in
/// seconds. Both are the states polls wait through: activation and
/// enrollment rewrite the replica when they land, but only in the colo
/// that served them, so a short validity caps how stale another colo
/// can stay.
const UNSETTLED_VALIDITY: u64 = 30;

/// A remembered customer-row read: the row, or its recorded absence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CachedCustomer {
    /// Value-shape version; see [`VERSION`].
    pub v: u32,
    /// Absolute expiry: a reader at or past this moment revalidates.
    pub not_after: u64,
    /// The row as D1 answered it; `None` records that no customer
    /// holds the asked-for key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub customer: Option<Customer>,
}

impl CachedCustomer {
    /// Record a row read at `now`, with validity for its state.
    pub fn record(customer: Option<Customer>, seed: &str, now: u64) -> Self {
        let base = match &customer {
            Some(customer) if customer.status != CustomerStatus::Registered => SETTLED_VALIDITY,
            _ => UNSETTLED_VALIDITY,
        };
        Self {
            v: VERSION,
            not_after: now + base + jitter(seed, now, base / 5),
            customer,
        }
    }

    /// Whether this record may still be used at `now`.
    pub fn fresh(&self, now: u64) -> bool {
        now < self.not_after
    }

    /// The record as its stored JSON.
    pub fn encode(&self) -> String {
        serde_json::to_string(self).expect("customer record serializes")
    }

    /// Read a stored value back; `None` — a miss — for anything this
    /// build does not recognize.
    pub fn decode(text: &str) -> Option<Self> {
        serde_json::from_str::<Self>(text)
            .ok()
            .filter(|cached| cached.v == VERSION)
    }

    /// Seconds the backing store should retain this value. At least
    /// KV's 60-second minimum; `not_after` inside the value governs.
    pub fn retention(&self, now: u64) -> u64 {
        (self.not_after.saturating_sub(now)).max(60)
    }
}

/// The KV key for the row read by account DID.
pub fn did_key(did: &str) -> String {
    format!("customer/{did}")
}

/// The KV key for the row read by email address, normalized so two
/// spellings of one address share an entry.
pub fn email_key(address: &str) -> String {
    format!("address/{}", normalize_email(address))
}

/// Read a fresh record from KV. `None` on a miss, a stale or
/// unrecognized value, or a read error — all of which the caller
/// answers by falling through to D1, exactly as it did before the
/// replica existed.
#[cfg(target_arch = "wasm32")]
pub async fn load(kv: &worker::kv::KvStore, key: &str, now: u64) -> Option<CachedCustomer> {
    match kv.get(key).text().await {
        Ok(Some(text)) => CachedCustomer::decode(&text).filter(|cached| cached.fresh(now)),
        Ok(None) => None,
        Err(err) => {
            worker::console_error!("customer replica unreadable at {key}: {err}");
            None
        }
    }
}

/// Record what D1 answered for the asked-for key. A present row is
/// written under both its access paths — the probe's DID key and the
/// lookup's address key — so either endpoint warms the other; a
/// recorded absence is a fact only about the key that was asked.
/// Best-effort: a failed write costs at most one more D1 read.
#[cfg(target_arch = "wasm32")]
pub async fn backfill(kv: &worker::kv::KvStore, asked: &str, customer: Option<Customer>, now: u64) {
    match customer {
        Some(customer) => replicate(kv, &customer, now).await,
        None => {
            let record = CachedCustomer::record(None, asked, now);
            save(kv, asked, &record, now).await;
        }
    }
}

/// Write the row through under both its keys. Called by the read path
/// on a backfill and by the registration commands after their D1
/// commit, so a state change propagates as itself.
#[cfg(target_arch = "wasm32")]
pub async fn replicate(kv: &worker::kv::KvStore, customer: &Customer, now: u64) {
    let record = CachedCustomer::record(Some(customer.clone()), &customer.account, now);
    save(kv, &did_key(&customer.account), &record, now).await;
    save(kv, &email_key(&customer.email), &record, now).await;
}

/// Drop the DID key, forcing the next probe through authoritative D1.
/// The address key is not reachable from a DID alone once the row is
/// gone; it rides out its own validity instead.
#[cfg(target_arch = "wasm32")]
pub async fn forget(kv: &worker::kv::KvStore, did: &str) {
    if let Err(err) = kv.delete(&did_key(did)).await {
        worker::console_error!("customer replica for {did} not dropped: {err}");
    }
}

#[cfg(target_arch = "wasm32")]
async fn save(kv: &worker::kv::KvStore, key: &str, record: &CachedCustomer, now: u64) {
    let write = kv
        .put(key, record.encode())
        .map(|put| put.expiration_ttl(record.retention(now)));
    let written = match write {
        Ok(put) => put.execute().await.map_err(|err| err.to_string()),
        Err(err) => Err(err.to_string()),
    };
    if let Err(err) = written {
        worker::console_error!("customer replica at {key} not written: {err}");
    }
}

/// Deterministic spread in `0..=range`, so records written together do
/// not all expire together.
fn jitter(seed: &str, now: u64, range: u64) -> u64 {
    let mut hasher = DefaultHasher::new();
    seed.hash(&mut hasher);
    now.hash(&mut hasher);
    match range {
        0 => 0,
        range => hasher.finish() % (range + 1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn customer(status: CustomerStatus) -> Customer {
        Customer {
            account: "did:key:zAlice".into(),
            email: "alice@example.com".into(),
            ledger: None,
            status,
            plan: "trial".into(),
            verified_at: 7,
            terms_version: Some("1".into()),
        }
    }

    #[test]
    fn it_round_trips_a_row_and_its_absence() {
        let present = CachedCustomer::record(Some(customer(CustomerStatus::Active)), "k", 1_000);
        assert_eq!(CachedCustomer::decode(&present.encode()), Some(present));
        let absent = CachedCustomer::record(None, "k", 1_000);
        let decoded = CachedCustomer::decode(&absent.encode()).expect("decodes");
        assert_eq!(decoded.customer, None);
    }

    #[test]
    fn it_keeps_unsettled_states_short() {
        let active = CachedCustomer::record(Some(customer(CustomerStatus::Active)), "k", 1_000);
        let registered =
            CachedCustomer::record(Some(customer(CustomerStatus::Registered)), "k", 1_000);
        let absent = CachedCustomer::record(None, "k", 1_000);
        assert!(registered.not_after < active.not_after);
        assert!(absent.not_after < active.not_after);
        assert!(!registered.fresh(1_000 + UNSETTLED_VALIDITY + UNSETTLED_VALIDITY / 5 + 1));
        assert!(active.fresh(1_000 + UNSETTLED_VALIDITY + UNSETTLED_VALIDITY / 5 + 1));
    }

    #[test]
    fn it_treats_an_unknown_version_as_a_miss() {
        let mut recorded = CachedCustomer::record(None, "k", 1_000);
        recorded.v = VERSION + 1;
        assert_eq!(CachedCustomer::decode(&recorded.encode()), None);
        assert_eq!(CachedCustomer::decode("not json"), None);
    }

    #[test]
    fn it_normalizes_the_address_key() {
        assert_eq!(
            email_key("Alice@Example.COM"),
            email_key("alice@example.com")
        );
    }

    #[test]
    fn it_retains_at_least_the_kv_minimum() {
        let absent = CachedCustomer::record(None, "k", 1_000);
        assert!(absent.retention(1_000) >= 60);
    }
}
