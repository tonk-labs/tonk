//! [`CustodiedSeed`] — a space or invite seed sealed to an account.

use dialog_artifacts::Entity;
use dialog_query::Concept;
use dialog_varsig::Did;
use serde::Serialize;

use crate::domain::custody::{Kind, Recipient, Sealed, Subject};
use crate::prelude::*;

/// A seed sealed to one recipient for one subject, in the account space.
///
/// The `this` entity is content-derived from the `(subject, recipient)`
/// pair, so re-sealing the same seed to the same account converges on
/// one row, while rotation adds a row for the new recipient and retracts
/// the old one rather than overwriting in place. Sealing a space seed to
/// another account (an admin as recovery custodian) is another row with
/// another recipient, no schema change.
///
/// `subject`, `kind`, and `recipient` repeat the hash inputs as
/// queryable attributes: rotation enumerates everything sealed to the
/// old recipient, and a recovering device finds the seed for a space
/// without knowing the entity. The sealed bytes are a
/// `tonk_identity::sealed::Sealed` envelope, which binds the recipient
/// and subject DIDs as associated data, so a row cannot be re-pointed.
///
/// The seed sits beside the ownership delegation rather than on it: the
/// delegation says who may act for the space, the seed says how to
/// re-issue it. Design: `plan/authority-facts.md`, "Wrapped keys".
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct CustodiedSeed {
    /// The row's entity. Derived from `(subject, recipient)`.
    pub this: Entity,
    /// The DID the seed derives.
    pub subject: Subject,
    /// What the subject is: [`SeedKind::Space`] or [`SeedKind::Invite`].
    pub kind: Kind,
    /// The X25519 `did:key` the seed is sealed to.
    pub recipient: Recipient,
    /// The sealed envelope bytes.
    pub sealed: Sealed,
}

/// What a custodied seed derives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeedKind {
    /// A space's signing key.
    Space,
    /// An invite principal's signing key.
    Invite,
}

impl SeedKind {
    /// `kind` URI for a space seed.
    pub const SPACE: &'static str = "tonk:space";
    /// `kind` URI for an invite seed.
    pub const INVITE: &'static str = "tonk:invite";

    /// The URI this kind is stored as.
    pub fn uri(self) -> &'static str {
        match self {
            Self::Space => Self::SPACE,
            Self::Invite => Self::INVITE,
        }
    }

    /// The stored [`Kind`]. The URIs are constants, so the parse cannot
    /// fail.
    pub fn kind(self) -> Kind {
        Kind(self.uri().parse().expect("a constant kind URI parses"))
    }
}

/// Hash input for [`CustodiedSeed::this`]. Single-variant enum tags the
/// CBOR encoding with the concept name so equal field data under a
/// different concept hashes differently.
#[derive(Debug, Clone, Serialize)]
enum This<'a> {
    CustodiedSeed {
        subject: &'a Did,
        recipient: &'a Did,
    },
}

impl CustodiedSeed {
    /// A seed for `subject`, sealed to `recipient`.
    pub fn new(subject: Did, kind: SeedKind, recipient: Did, sealed: Vec<u8>) -> Self {
        Self {
            this: Entity::of(&This::CustodiedSeed {
                subject: &subject,
                recipient: &recipient,
            }),
            subject: Subject(subject.this()),
            kind: kind.kind(),
            recipient: Recipient(recipient.this()),
            sealed: Sealed(sealed),
        }
    }

    /// The row's entity.
    pub fn this(&self) -> &Entity {
        &self.this
    }
}

impl AsRef<Entity> for CustodiedSeed {
    fn as_ref(&self) -> &Entity {
        &self.this
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use dialog_operator::helpers;
    use dialog_query::{Output as _, Query, Term};
    use dialog_varsig::did;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    #[dialog_common::test]
    fn it_derives_the_same_entity_for_the_same_subject_and_recipient() {
        let a = CustodiedSeed::new(did!("test:s"), SeedKind::Space, did!("test:r"), vec![1]);
        let b = CustodiedSeed::new(did!("test:s"), SeedKind::Space, did!("test:r"), vec![2]);
        assert_eq!(a.this, b.this);
    }

    #[dialog_common::test]
    fn it_derives_different_entities_per_recipient_and_subject() {
        let base = CustodiedSeed::new(did!("test:s"), SeedKind::Space, did!("test:r1"), vec![]);
        let other_recipient =
            CustodiedSeed::new(did!("test:s"), SeedKind::Space, did!("test:r2"), vec![]);
        let other_subject =
            CustodiedSeed::new(did!("test:s2"), SeedKind::Space, did!("test:r1"), vec![]);
        assert_ne!(base.this, other_recipient.this);
        assert_ne!(base.this, other_subject.this);
    }

    #[dialog_common::test]
    fn it_reflects_the_inputs_as_attributes() {
        let row = CustodiedSeed::new(
            did!("test:space"),
            SeedKind::Invite,
            did!("test:recipient"),
            vec![7, 8],
        );
        assert_eq!(row.subject.0.to_string(), "did:test:space");
        assert_eq!(row.recipient.0.to_string(), "did:test:recipient");
        assert_eq!(row.kind.0.to_string(), SeedKind::INVITE);
        assert_eq!(row.sealed.0, vec![7, 8]);
    }

    #[dialog_common::test]
    async fn it_round_trips_through_a_branch_and_finds_rows_by_recipient() -> Result<()> {
        let (operator, profile) = helpers::test_operator_with_profile().await;
        let repository = helpers::test_repo(&operator, &profile).await;
        let branch = repository.branch("main").open().perform(&operator).await?;

        let old = did!("test:old-recipient");
        let new = did!("test:new-recipient");
        branch
            .transaction()
            .assert(CustodiedSeed::new(
                did!("test:space-a"),
                SeedKind::Space,
                old.clone(),
                vec![1],
            ))
            .assert(CustodiedSeed::new(
                did!("test:space-b"),
                SeedKind::Space,
                old.clone(),
                vec![2],
            ))
            .assert(CustodiedSeed::new(
                did!("test:space-a"),
                SeedKind::Space,
                new.clone(),
                vec![3],
            ))
            .commit()
            .perform(&operator)
            .await?;

        let sealed_to_old: Vec<CustodiedSeed> = branch
            .query()
            .select(Query::<CustodiedSeed> {
                this: Term::var("this"),
                subject: Term::var("subject"),
                kind: Term::var("kind"),
                recipient: Term::from(Recipient(old.this())),
                sealed: Term::var("sealed"),
            })
            .perform(&operator)
            .try_vec()
            .await?;
        let mut subjects: Vec<String> = sealed_to_old
            .iter()
            .map(|row| row.subject.0.to_string())
            .collect();
        subjects.sort();
        assert_eq!(subjects, vec!["did:test:space-a", "did:test:space-b"]);

        let for_space_a: Vec<CustodiedSeed> = branch
            .query()
            .select(Query::<CustodiedSeed> {
                this: Term::var("this"),
                subject: Term::from(Subject(did!("test:space-a").this())),
                kind: Term::var("kind"),
                recipient: Term::var("recipient"),
                sealed: Term::var("sealed"),
            })
            .perform(&operator)
            .try_vec()
            .await?;
        assert_eq!(for_space_a.len(), 2, "one row per recipient");
        Ok(())
    }
}
