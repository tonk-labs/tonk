//! Operational metadata for a configured remote.

#![allow(missing_docs)]

use dialog_artifacts::Entity;
use dialog_query::Concept;

use crate::Remote;
use crate::domain::remote_execution::RevocationUrl;

/// Revocation-routing metadata stored beside a [`Remote`].
///
/// `this` is exactly the remote entity, preserving the original `Remote`
/// schema and making missing metadata explicit for legacy records.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct RemoteExecution {
    /// The associated remote entity.
    pub this: Entity,
    /// Explicit immutable-artifact relay.
    pub revocation_url: RevocationUrl,
}

impl RemoteExecution {
    /// Build execution metadata for a remote.
    pub fn new(remote: &Remote, revocation_url: &str) -> Self {
        Self {
            this: remote.this.clone(),
            revocation_url: RevocationUrl(revocation_url.to_owned()),
        }
    }
}
