//! The device-local list of profiles this browser can open: one entity per
//! profile, carrying the storage handle to open it with.
//!
//! One field, because one is all that is original here. Everything a switcher
//! row shows about a profile — its display name, the address it registered —
//! already lives on that profile's own account branch and is read from there.
//! Copies were kept here once and could only go stale: nothing invalidated
//! them, so a name changed on another device lingered until that profile was
//! next activated on this one.
//!
//! Nothing here says whether a profile is SIGNED IN, deliberately. That is a
//! delegation that exists and verifies — the `account -> profile` device
//! link, whose audience is the profile and whose issuer is an account that is
//! itself active. Signing out retracts it, so the authority is gone rather
//! than merely unrecorded, and the question is answered from the proof
//! instead of from a stamp that can disagree with it.
//!
//! Facts rather than one serialized blob so concurrent writers merge per
//! entity instead of racing a whole-roster read-modify-write.

// The `#[derive(Concept)]` macro generates helper types without doc
// comments; suppress `missing_docs` like the sibling concept modules.
#![allow(missing_docs)]

use dialog_artifacts::Entity;
use dialog_query::Concept;
use dialog_varsig::Did;

use crate::domain::roster::Name;
use crate::prelude::*;

/// One profile this device can open, keyed by the profile's own DID.
///
/// The storage name is not part of the identity and does not need to be: a
/// profile is opened by name (`Profile::open(name)`), so the name determines
/// the DID and the pair would carry no more information than the DID alone.
/// The name is a field here — what to open, not who this is.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct DeviceProfile {
    /// The profile's DID.
    pub this: Entity,
    /// Storage name the profile opens under.
    pub name: Name,
}

impl DeviceProfile {
    /// The entry for `profile`, opened under the storage name `name`.
    pub fn new(profile: &Did, name: impl Into<String>) -> Self {
        Self {
            this: profile.this(),
            name: Name(name.into()),
        }
    }

    /// The entity of the entry for `profile`.
    pub fn entity(profile: &Did) -> Entity {
        profile.this()
    }

    /// The entry's entity.
    pub fn this(&self) -> &Entity {
        &self.this
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use dialog_operator::helpers;
    use dialog_query::{Output as _, Query, Term};
    use dialog_varsig::did;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;

    use super::*;

    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    /// An entry is found by the profile it names, and carries the handle to
    /// open it with.
    #[dialog_common::test]
    async fn it_finds_a_profile_entry_by_its_did() -> Result<()> {
        let (operator, profile) = helpers::test_operator_with_profile().await;
        let repository = helpers::test_repo(&operator, &profile).await;
        let branch = repository.branch("main").open().perform(&operator).await?;
        let subject = did!("test:profile-one");

        branch
            .transaction()
            .assert(DeviceProfile::new(&subject, "profile-1"))
            .commit()
            .perform(&operator)
            .await?;

        let rows: Vec<DeviceProfile> = branch
            .query()
            .select(Query::<DeviceProfile> {
                this: Term::from(subject.this()),
                name: Term::var("name"),
            })
            .perform(&operator)
            .try_vec()
            .await?;

        assert_eq!(rows.len(), 1, "one entry for one profile");
        assert_eq!(rows[0].name.0, "profile-1");
        Ok(())
    }

    /// Two profiles are two entries, and a query with an open subject
    /// returns both — the switcher's own read.
    #[dialog_common::test]
    async fn it_lists_every_profile_this_device_can_open() -> Result<()> {
        let (operator, profile) = helpers::test_operator_with_profile().await;
        let repository = helpers::test_repo(&operator, &profile).await;
        let branch = repository.branch("main").open().perform(&operator).await?;

        branch
            .transaction()
            .assert(DeviceProfile::new(&did!("test:profile-one"), "profile-1"))
            .assert(DeviceProfile::new(&did!("test:profile-two"), "profile-2"))
            .commit()
            .perform(&operator)
            .await?;

        let rows: Vec<DeviceProfile> = branch
            .query()
            .select(Query::<DeviceProfile> {
                this: Term::var("this"),
                name: Term::var("name"),
            })
            .perform(&operator)
            .try_vec()
            .await?;

        let mut names: Vec<String> = rows.into_iter().map(|row| row.name.0).collect();
        names.sort();
        assert_eq!(names, vec!["profile-1", "profile-2"]);
        Ok(())
    }

    /// Re-recording a profile whose storage handle changed updates the entry
    /// in place rather than adding a second one: the entity is the profile,
    /// so `name` is cardinality-one on it.
    #[dialog_common::test]
    async fn it_keeps_one_entry_per_profile_when_the_handle_changes() -> Result<()> {
        let (operator, profile) = helpers::test_operator_with_profile().await;
        let repository = helpers::test_repo(&operator, &profile).await;
        let branch = repository.branch("main").open().perform(&operator).await?;
        let subject = did!("test:profile-one");

        branch
            .transaction()
            .assert(DeviceProfile::new(&subject, "profile-1"))
            .commit()
            .perform(&operator)
            .await?;
        branch
            .transaction()
            .assert(DeviceProfile::new(&subject, "profile-renamed"))
            .commit()
            .perform(&operator)
            .await?;

        let rows: Vec<DeviceProfile> = branch
            .query()
            .select(Query::<DeviceProfile> {
                this: Term::from(subject.this()),
                name: Term::var("name"),
            })
            .perform(&operator)
            .try_vec()
            .await?;

        assert_eq!(rows.len(), 1, "one profile is one entry");
        assert_eq!(rows[0].name.0, "profile-renamed");
        Ok(())
    }

    /// The entity is the profile's own DID, not a hash of anything here.
    #[dialog_common::test]
    fn it_keys_the_entry_on_the_profile_did() {
        let subject = did!("test:profile-one");
        let entry = DeviceProfile::new(&subject, "profile-1");
        assert_eq!(*entry.this(), subject.this());
        assert_eq!(DeviceProfile::entity(&subject), subject.this());
    }
}
