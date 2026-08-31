//! [`DeviceLink`] — an `account -> profile` powerline, as a device list
//! presents it.

// The `#[derive(Concept)]` macro generates helper types without doc
// comments; suppress `missing_docs` like the sibling concept modules.
#![allow(missing_docs)]

use dialog_artifacts::{ArtifactSelector, Entity, Value};
use dialog_capability::{Fork, Provider};
use dialog_common::ConditionalSync;
use dialog_effects::archive::{Get, Put};
use dialog_effects::authority::Identify;
use dialog_effects::memory::Resolve;
use dialog_query::{Concept, EvaluationError, Output as _, Query, Term};
use dialog_repository::{Branch, DELEGATION_AUDIENCE, RemoteSite};
use futures_util::StreamExt as _;

use crate::domain::device::{CreatedAt, Reason, Title};

/// The reason recorded on a link minted for a device.
pub const DEVICE_LINK: &str = "case:device-link";

/// The stored [`Reason`] for a device link. The URI is a constant, so
/// the parse cannot fail.
pub fn device_link_reason() -> Reason {
    Reason(DEVICE_LINK.parse().expect("a constant reason URI parses"))
}

/// A device authorization: the label and creation time of an
/// `account -> profile` delegation.
///
/// # Why this has no identifying fields
///
/// Every other concept derives `this` from the data that identifies it.
/// This one takes the entity as given, because the identity already
/// exists: dialog stores a retained delegation under
/// `Entity::from_blob(hash)` and decomposes issuer, audience, subject,
/// command, and expiration onto it. This concept adds the fields dialog
/// does not carry, onto the entity dialog already made.
///
/// That is deliberate. The delegation IS the authorization — it is what
/// confers the authority and it is signed — so a separate record keyed
/// by device DID would be a second source of truth that could disagree
/// with the proof. It also means revoking the delegation takes this row
/// with it: a device cannot linger in a list after losing its authority.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct DeviceLink {
    /// The delegation's entity — its blob hash, as dialog keyed it.
    pub this: Entity,
    /// When the link was minted, unix seconds.
    pub created_at: CreatedAt,
    /// Human label for the device.
    pub title: Title,
    /// Why the delegation exists — [`DEVICE_LINK`] for a device.
    pub reason: Reason,
}

impl DeviceLink {
    /// Describe the delegation stored at `entity` as a device link.
    ///
    /// `entity` comes from retaining the chain — dialog returns the
    /// entities it wrote — so this never derives a hash of its own and
    /// cannot describe a delegation that was never stored.
    pub fn new(entity: Entity, title: impl Into<String>, created_at: u64) -> Self {
        Self {
            this: entity,
            created_at: CreatedAt(created_at),
            title: Title(title.into()),
            reason: device_link_reason(),
        }
    }
}

/// This account's device links and their audiences, from the account
/// branch's own facts — the one query the worker's device list and the
/// CLI's `account devices` both run.
///
/// Dialog decomposes issuer/audience onto each retained delegation's
/// entity, and [`DeviceLink`] adds the label and creation time, so every
/// row is derivable locally. A link whose audience fact is missing is
/// skipped: without an audience there is no device to attribute the row
/// to.
pub async fn device_links<Env>(
    account: &Branch,
    env: &Env,
) -> Result<Vec<(DeviceLink, String)>, EvaluationError>
where
    Env: Provider<Get>
        + Provider<Put>
        + Provider<Resolve>
        + Provider<Identify>
        + Provider<Fork<RemoteSite, Get>>
        + Provider<Fork<RemoteSite, Resolve>>
        + ConditionalSync
        + 'static,
{
    let links: Vec<DeviceLink> = account
        .query()
        .select(Query::<DeviceLink> {
            this: Term::var("this"),
            created_at: Term::var("created_at"),
            title: Term::var("title"),
            reason: Term::var("reason"),
        })
        .perform(env)
        .try_vec()
        .await?;

    let mut devices = Vec::with_capacity(links.len());
    for link in links {
        let Some(did) = delegation_audience(account, &link.this, env).await else {
            continue;
        };
        devices.push((link, did));
    }
    Ok(devices)
}

/// The audience DID dialog recorded for a retained delegation.
///
/// `dialog.ucan/audience` is written onto the delegation's own entity
/// when the chain is retained, so this reads the device's identity from
/// the same record that carries its label — no second source to drift.
async fn delegation_audience<Env>(account: &Branch, entity: &Entity, env: &Env) -> Option<String>
where
    Env: Provider<Get>
        + Provider<Put>
        + Provider<Resolve>
        + Provider<Identify>
        + Provider<Fork<RemoteSite, Get>>
        + Provider<Fork<RemoteSite, Resolve>>
        + ConditionalSync
        + 'static,
{
    let selector = ArtifactSelector::new()
        .the(DELEGATION_AUDIENCE.parse().ok()?)
        .of(entity.clone());
    let facts = account
        .claims()
        .select(selector)
        .perform(env)
        .await
        .ok()?
        .collect::<Vec<_>>()
        .await;
    for fact in facts.into_iter().flatten() {
        if let Ok(Value::String(did)) = fact.value() {
            return Some(did.to_string());
        }
    }
    None
}
