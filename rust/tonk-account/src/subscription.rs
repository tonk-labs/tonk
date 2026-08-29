//! Operator commands on a subscription.
//!
//! All three take the service's own DID as subject: they are the
//! service's decisions about a customer, not anything a customer
//! authorizes. Only a key the service delegated to can invoke them,
//! which is what an operator tool holds.
//!
//! They are three different things, and the row records them separately:
//!
//! | | Data | Row | Comes back |
//! |---|---|---|---|
//! | Suspend | kept | kept | on resume |
//! | Archive | dropped | kept, for billing | on re-provisioning |
//! | Delete | dropped | purged | no |
//!
//! Deletion is the customer's own request and lives in `deletion`.

use dialog_capability::{Attenuate, Effect};
use dialog_effects::Use;
use dialog_varsig::Did;
use serde::{Deserialize, Serialize};

/// `/use/put/subscription/suspend` — withdraw service from one
/// subscription, and `/use/put/subscription/resume` to restore it.
///
/// The subject is the service's own DID, because suspension is the
/// service's decision about a customer rather than anything the customer
/// authorizes. Only a key the service delegated to can invoke it, which
/// is what an operator tool holds.
///
/// Distinct from `customer.status = 'Suspended'`, which withdraws
/// service from everything that customer provides. This is one space.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Attenuate)]
pub struct Suspend {
    /// The subscription to suspend, named by the DID it is for.
    pub consumer: Did,
    /// Machine-readable reason, recorded on the row and reported in the
    /// refusal so a client can tell one suspension from another.
    pub code: String,
    /// What to tell a person. Recorded alongside the code.
    pub reason: String,
    /// When the suspension lifts on its own. Absent means indefinite,
    /// matching `suspend_until_at` being null.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until: Option<u64>,
}

impl Effect for Suspend {
    type Of = Use;
    type Output = ();

    fn command() -> &'static str {
        "put/subscription/suspend"
    }
}

/// `/use/put/subscription/resume` — restore a suspended subscription.
///
/// Clears the code, the message, and the deadline together: a
/// half-cleared suspension is a row nothing can explain.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Attenuate)]
pub struct Resume {
    /// The subscription to restore.
    pub consumer: Did,
}

impl Effect for Resume {
    type Of = Use;
    type Output = ();

    fn command() -> &'static str {
        "put/subscription/resume"
    }
}

/// `/use/put/subscription/archive` — stop carrying a subscription's
/// data.
///
/// Distinct from both of the above. Suspension withholds service and
/// keeps the data; archival deletes the data and keeps the row, because
/// what it accrued still has to be billed
/// (`plan/Access metering.md` §10). Distinct from deletion too: that is
/// the customer's own request, purges the row, and does not come back.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Attenuate)]
pub struct Archive {
    /// The subscription whose data is being dropped.
    pub consumer: Did,
}

impl Effect for Archive {
    type Of = Use;
    type Output = ();

    fn command() -> &'static str {
        "put/subscription/archive"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dialog_capability::Subject;
    use dialog_capability::did;

    /// The paths an operator tool delegates and the service dispatches
    /// on. Asserted because they are wire: a rename here is a protocol
    /// change, not a refactor.
    #[test]
    fn it_names_the_suspension_commands() {
        let suspend = Subject::from(did!("web:network.tonk"))
            .attenuate(Use)
            .invoke(Suspend {
                consumer: "did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK"
                    .parse()
                    .unwrap(),
                code: "unpaid".into(),
                reason: "the subscription is past due".into(),
                until: None,
            });
        assert_eq!(suspend.ability(), "/use/put/subscription/suspend");

        let resume = Subject::from(did!("web:network.tonk"))
            .attenuate(Use)
            .invoke(Resume {
                consumer: "did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK"
                    .parse()
                    .unwrap(),
            });
        assert_eq!(resume.ability(), "/use/put/subscription/resume");

        let archive = Subject::from(did!("web:network.tonk"))
            .attenuate(Use)
            .invoke(Archive {
                consumer: "did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK"
                    .parse()
                    .unwrap(),
            });
        assert_eq!(archive.ability(), "/use/put/subscription/archive");
    }
}
