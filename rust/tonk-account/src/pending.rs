//! Work deferred until the account confirms its email.
//!
//! The access service serves nothing, and provisions nothing, for a
//! customer that has enrolled but not yet clicked the emailed
//! activation link (`plan/account-activation-gate.md`). Everything a
//! client would otherwise do in that window — provisioning the custody
//! space, publishing the sealed account secret, provisioning a space
//! created before the email arrived — is recorded here instead and
//! replayed once the customer goes `Active`.
//!
//! Order is the queue's only invariant: a custody cell may only be
//! published after that custody DID has been provisioned, and replaying
//! entries in the order they were recorded gives that for free. A drain
//! therefore stops at the first entry it cannot complete and leaves the
//! rest queued, rather than skipping ahead.

use serde::{Deserialize, Serialize};

/// One deferred act.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PendingWork {
    /// Provision a consumer under this account: a `/provider/add`
    /// carrying the consumer's own consent.
    #[serde(rename_all = "camelCase")]
    Provision {
        /// The consumer being provisioned.
        consumer: String,
        /// Hex-encoded consent delegation chain the consumer minted.
        consent_hex: String,
        /// `space` or `custody`; absent means `space`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        consumer_kind: Option<String>,
    },
    /// Publish a sealed account secret into a custody space's cell.
    ///
    /// The ceremony that sealed the envelope also pre-signed the
    /// publish invocation — the one moment the PRF-derived custody key
    /// is in hand — with a bounded long expiration, so the worker can
    /// drain this with no page, no assertion, and no button the moment
    /// activation and provisioning land. The signature covers exactly
    /// this cell and this content's checksum: least authority, held as
    /// queued work.
    ///
    /// The envelope is already sealed under the passkey's KEK when it
    /// arrives here, so what waits is ciphertext rather than key
    /// material — the same bytes that would otherwise sit in the custody
    /// cell, which is itself storage the service can read.
    #[serde(rename_all = "camelCase")]
    PublishCustody {
        /// The custody space's DID: the invocation's subject, and what
        /// must be provisioned before this can be served.
        custody: String,
        /// Hex-encoded sealed envelope, the content to publish.
        sealed_hex: String,
        /// Hex-encoded pre-signed `/use/put/memory/cell` invocation. An
        /// entry from before invocations were queued decodes with an
        /// empty string and is void: dropped with a log rather than
        /// blocking the queue forever.
        #[serde(default)]
        invocation_hex: String,
    },
}

impl PendingWork {
    /// The subject this entry acts on, for logging and de-duplication.
    pub fn subject(&self) -> &str {
        match self {
            Self::Provision { consumer, .. } => consumer,
            Self::PublishCustody { custody, .. } => custody,
        }
    }
}

/// The recorded queue, oldest first.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PendingQueue(pub Vec<PendingWork>);

impl PendingQueue {
    /// Whether anything is waiting.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// How many entries are waiting.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Append `work`, unless an identical entry is already queued.
    ///
    /// Re-running a ceremony that was interrupted must not grow the
    /// queue without bound, and replaying the same act twice is at best
    /// wasted round trips.
    pub fn push(&mut self, work: PendingWork) {
        if !self.0.contains(&work) {
            self.0.push(work);
        }
    }

    /// Append a related batch in its supplied order, suppressing exact work
    /// already present without allowing a missing prerequisite to land behind
    /// a surviving later entry. Callers serialize the queue once afterwards.
    pub fn push_all(&mut self, work: impl IntoIterator<Item = PendingWork>) {
        let batch = work.into_iter().collect::<Vec<_>>();
        for (index, entry) in batch.iter().cloned().enumerate() {
            if self.0.contains(&entry) {
                continue;
            }
            let insert_at = batch[index + 1..]
                .iter()
                .filter_map(|later| self.0.iter().position(|queued| queued == later))
                .min();
            if let Some(insert_at) = insert_at {
                self.0.insert(insert_at, entry);
            } else {
                self.0.push(entry);
            }
        }
    }

    /// The entries in replay order.
    pub fn entries(&self) -> &[PendingWork] {
        &self.0
    }

    /// Drop the first `count` entries, the ones a drain completed.
    pub fn retain_after(&mut self, count: usize) {
        self.0.drain(..count.min(self.0.len()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provision(consumer: &str) -> PendingWork {
        PendingWork::Provision {
            consumer: consumer.to_string(),
            consent_hex: "aa".to_string(),
            consumer_kind: Some("custody".to_string()),
        }
    }

    #[test]
    fn it_keeps_recorded_order_and_ignores_duplicates() {
        let mut queue = PendingQueue::default();
        queue.push(provision("did:key:zCustody"));
        queue.push(PendingWork::PublishCustody {
            custody: "did:key:zCustody".to_string(),
            sealed_hex: "bb".to_string(),
            invocation_hex: "c0de".to_string(),
        });
        queue.push(provision("did:key:zCustody"));

        assert_eq!(queue.len(), 2, "an identical entry is not queued twice");
        assert_eq!(queue.entries()[0].subject(), "did:key:zCustody");
        assert!(
            matches!(queue.entries()[1], PendingWork::PublishCustody { .. }),
            "the publish must stay behind the provision that makes it servable"
        );
    }

    #[test]
    fn it_repairs_a_surviving_publish_when_the_complete_batch_is_replayed() {
        let provision = provision("did:key:zCustody");
        let publish = PendingWork::PublishCustody {
            custody: "did:key:zCustody".to_string(),
            sealed_hex: "bb".to_string(),
            invocation_hex: "c0de".to_string(),
        };
        let mut queue = PendingQueue(vec![publish.clone()]);

        queue.push_all([provision.clone(), publish.clone()]);
        queue.push_all([provision.clone(), publish.clone()]);

        assert_eq!(queue.entries(), &[provision, publish]);
    }

    #[test]
    fn it_clears_only_the_entries_a_drain_completed() {
        let mut queue = PendingQueue::default();
        queue.push(provision("did:key:zOne"));
        queue.push(provision("did:key:zTwo"));
        queue.push(provision("did:key:zThree"));

        // A drain that completed one entry and stopped at the second
        // leaves the rest queued, in order.
        queue.retain_after(1);
        assert_eq!(queue.len(), 2);
        assert_eq!(queue.entries()[0].subject(), "did:key:zTwo");
        assert_eq!(queue.entries()[1].subject(), "did:key:zThree");

        queue.retain_after(99);
        assert!(queue.is_empty(), "clearing past the end empties the queue");
    }

    #[test]
    fn it_round_trips_through_the_recorded_form() {
        let mut queue = PendingQueue::default();
        queue.push(provision("did:key:zCustody"));
        queue.push(PendingWork::PublishCustody {
            custody: "did:key:zCustody".to_string(),
            sealed_hex: "bb".to_string(),
            invocation_hex: "c0de".to_string(),
        });

        let bytes = serde_json::to_vec(&queue).expect("queue serializes");
        let restored: PendingQueue = serde_json::from_slice(&bytes).expect("queue deserializes");
        assert_eq!(restored, queue);
    }
}
