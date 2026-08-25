//! The device-local roster of profiles this browser knows: one entity per
//! profile storage name, with the attachment and email as stamps.
//!
//! The switcher has to describe profiles it has not opened, and opening
//! each one just to render a row would cost key-material load per profile
//! per render. So each entry caches what a row needs and nothing more: the
//! storage name to activate, a display label, and the attachment that
//! decides between "Signed in" and "Local workspace". Identity fields keep
//! their one home in the account space; the label is a cache of it,
//! refreshed at the moments the worker already has the facts in hand.
//!
//! Facts rather than one serialized blob so concurrent writers merge per
//! entity instead of racing a whole-roster read-modify-write.

// The `#[derive(Concept)]` macro generates helper types without doc
// comments; suppress `missing_docs` like the sibling concept modules.
#![allow(missing_docs)]

use dialog_artifacts::Entity;
use dialog_query::Concept;
use dialog_varsig::Did;
use serde::Serialize;

use crate::domain::roster::{Account, Email, Label, Name, Provider};
use crate::prelude::*;

/// One profile this browser knows, keyed by its storage name.
///
/// The entity is content-derived from the storage name, so a refresh
/// from any moment (boot, link, rename, switch) converges on the same
/// entity. `label` is cardinality-one: the latest refresh wins.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct RosterProfile {
    /// The entry's entity. Derived from `name`.
    pub this: Entity,
    /// Storage name the profile opens under.
    pub name: Name,
    /// Display label as of the last refresh.
    pub label: Label,
}

/// Hash input for [`RosterProfile::this`]. Single-variant enum tags the
/// CBOR encoding with the concept name so equal field data under a
/// different concept hashes differently.
#[derive(Debug, Clone, Serialize)]
enum This<'a> {
    RosterProfile { name: &'a str },
}

impl RosterProfile {
    /// The entry for the profile stored under `name`, labelled `label`.
    pub fn new(name: String, label: String) -> Self {
        Self {
            this: Self::entity(&name),
            name: Name(name),
            label: Label(label),
        }
    }

    /// The entity of the entry for the profile stored under `name`.
    pub fn entity(name: &str) -> Entity {
        Entity::of(&This::RosterProfile { name })
    }

    /// The entry's entity.
    pub fn this(&self) -> &Entity {
        &self.this
    }
}

/// The account a roster profile is attached to, stamped onto its entity.
/// Absent for a local workspace: never signed in, or signed out. Signing
/// out retracts the stamp and keeps the entry, which is how a row is
/// demoted without deleting anything.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct RosterAccount {
    /// The roster entry being stamped.
    pub this: Entity,
    /// The account root the profile is attached to.
    pub account: Account,
    /// The attached provider base URL.
    pub provider: Provider,
}

impl RosterAccount {
    /// Stamp the entry at `entry` as attached to `account` through `provider`.
    pub fn new(entry: Entity, account: &Did, provider: String) -> Self {
        Self {
            this: entry,
            account: Account(account.this()),
            provider: Provider(provider),
        }
    }
}

/// The account email of a roster profile, stamped onto its entity.
/// Captured best-effort at link time and carried until the summary proxy
/// retires in favour of an account-space fact; may lag.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct RosterEmail {
    /// The roster entry being stamped.
    pub this: Entity,
    /// The account email.
    pub email: Email,
}

impl RosterEmail {
    /// Stamp the entry at `entry` with `email`.
    pub fn new(entry: Entity, email: String) -> Self {
        Self {
            this: entry,
            email: Email(email),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dialog_varsig::did;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    #[dialog_common::test]
    fn it_derives_the_same_entity_for_the_same_storage_name() {
        let a = RosterProfile::new("tonk".into(), "Alice".into());
        let b = RosterProfile::new("tonk".into(), "Renamed".into());
        assert_eq!(
            a.this, b.this,
            "a refresh converges on the entry it refreshes"
        );
        assert_eq!(RosterProfile::entity("tonk"), a.this);
    }

    #[dialog_common::test]
    fn it_derives_different_entities_for_different_storage_names() {
        let a = RosterProfile::new("tonk".into(), "Alice".into());
        let b = RosterProfile::new("tonk-0a".into(), "Alice".into());
        assert_ne!(a.this, b.this);
    }

    #[dialog_common::test]
    fn it_stamps_the_entry_with_its_attachment() {
        let entry = RosterProfile::entity("tonk");
        let account = did!("key:zAccount");
        let stamp = RosterAccount::new(entry.clone(), &account, "https://accounts.example".into());
        assert_eq!(stamp.this, entry);
        assert_eq!(stamp.account.0, account.this());
        assert_eq!(stamp.provider.0, "https://accounts.example");
        let email = RosterEmail::new(entry.clone(), "person@example.com".into());
        assert_eq!(email.this, entry);
    }
}
