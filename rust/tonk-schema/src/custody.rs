//! [`CustodiedSeed`] — a space or invite seed sealed to an account.

use dialog_artifacts::Entity;
use dialog_query::Concept;
use dialog_varsig::Did;
use serde::Serialize;

use crate::domain::custody::{Account, Cell, Kind, Recipient, Sealed, Subject};
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

/// A passkey's custody cell, recorded in the account space beside the
/// vault copy.
///
/// The cell is the account secret sealed under one passkey's KEK
/// (`tonk_identity::envelope::Envelope`) — ciphertext only a fresh
/// assertion of that passkey can open. The vault copy bootstraps a
/// brand-new browser, which has no profile branch yet; this row rides
/// the account's own sync, so every device already holding the profile
/// carries the recovery envelope too.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct CustodyCell {
    /// The custody space's DID — the passkey-derived principal.
    pub this: Entity,
    /// The account this cell recovers.
    pub account: Account,
    /// The sealed envelope bytes.
    pub cell: Cell,
}

impl CustodyCell {
    /// Record `cell` as `custody`'s envelope for `account`.
    pub fn new(custody: Did, account: Did, cell: Vec<u8>) -> Self {
        Self {
            this: custody.this(),
            account: Account(account.this()),
            cell: Cell(cell),
        }
    }
}

/// The outcome of one [`rotate`] pass: what moved, and what stayed
/// sealed to the old recipient with the reason it stayed.
#[derive(Debug, Default)]
pub struct Rotation {
    /// Subjects whose seeds are now sealed to the new recipient.
    pub rotated: Vec<Did>,
    /// Subjects that stayed, with why. A later pass picks them up.
    pub failures: Vec<(Did, String)>,
}

/// A [`rotate`] failure outside any one seed: the pass itself could not
/// run.
#[derive(Debug, thiserror::Error)]
pub enum RotateError {
    /// The custodied-seed rows could not be read.
    #[error("failed to read custodied seeds: {0}")]
    Read(String),
}

/// Rotate every custodied seed sealed to `old` onto `new`, on `branch`.
///
/// This is the shared rotation core: the worker runs it when a passkey
/// account arrives on a device that onboarded locally, and the CLI runs
/// it at `tonk account login` — one implementation, so the two adapters
/// cannot drift. Per seed the core opens with the old key, derives and
/// verifies the signer, seals to the new recipient, and hands the
/// adapter both the derived signer and the exact row replacement (the
/// old row to retract, the new row to assert). The adapter re-issues —
/// chains, prefixes, retention, provisioning — and commits the
/// replacement through its own branch handle, because its re-issue
/// writes advance the branch underneath any handle the core could hold.
///
/// Best-effort per seed: a seed that fails to open, verify, or reissue
/// is recorded in [`Rotation::failures`] and left sealed to the old
/// recipient, so a later pass resumes exactly where this one stopped.
pub async fn rotate<Env>(
    branch: &dialog_repository::Branch,
    old: &tonk_identity::sealed::EncryptionKey,
    new: &tonk_identity::sealed::RecipientKey,
    env: &Env,
    mut reissue: impl AsyncFnMut(
        SeedKind,
        dialog_credentials::Ed25519Signer,
        &CustodiedSeed,
        CustodiedSeed,
    ) -> Result<(), String>,
) -> Result<Rotation, RotateError>
where
    Env: dialog_capability::Provider<dialog_effects::archive::Get>
        + dialog_capability::Provider<dialog_effects::archive::Put>
        + dialog_capability::Provider<dialog_effects::archive::Import>
        + dialog_capability::Provider<dialog_effects::memory::Resolve>
        + dialog_capability::Provider<dialog_effects::memory::Publish>
        + dialog_capability::Provider<dialog_effects::authority::Identify>
        + dialog_capability::Provider<dialog_effects::authority::Attest>
        + dialog_capability::Provider<
            dialog_capability::Fork<dialog_repository::RemoteSite, dialog_effects::archive::Get>,
        > + dialog_capability::Provider<
            dialog_capability::Fork<dialog_repository::RemoteSite, dialog_effects::memory::Resolve>,
        > + dialog_common::ConditionalSync
        + 'static,
{
    use dialog_query::{Output as _, Query, Term};

    let old_recipient = old.recipient().did();
    let rows: Vec<CustodiedSeed> = branch
        .query()
        .select(Query::<CustodiedSeed> {
            this: Term::var("this"),
            subject: Term::var("subject"),
            kind: Term::var("kind"),
            recipient: Term::from(Recipient(old_recipient.this())),
            sealed: Term::var("sealed"),
        })
        .perform(env)
        .try_vec()
        .await
        .map_err(|error| RotateError::Read(format!("{error:?}")))?;

    let mut rotation = Rotation::default();
    for row in rows {
        let subject: Did = match row.subject.0.to_string().parse() {
            Ok(subject) => subject,
            Err(error) => {
                rotation.failures.push((
                    old_recipient.clone(),
                    format!("custodied subject is not a DID: {error}"),
                ));
                continue;
            }
        };
        match rotate_seed(old, new, &mut reissue, &subject, &row).await {
            Ok(()) => rotation.rotated.push(subject),
            Err(reason) => rotation.failures.push((subject, reason)),
        }
    }
    Ok(rotation)
}

async fn rotate_seed(
    old: &tonk_identity::sealed::EncryptionKey,
    new: &tonk_identity::sealed::RecipientKey,
    reissue: &mut impl AsyncFnMut(
        SeedKind,
        dialog_credentials::Ed25519Signer,
        &CustodiedSeed,
        CustodiedSeed,
    ) -> Result<(), String>,
    subject: &Did,
    row: &CustodiedSeed,
) -> Result<(), String> {
    use dialog_varsig::Principal as _;

    let kind = match row.kind.0.to_string().as_str() {
        SeedKind::SPACE => SeedKind::Space,
        SeedKind::INVITE => SeedKind::Invite,
        other => return Err(format!("unknown seed kind {other}")),
    };
    let sealed =
        tonk_identity::sealed::Sealed::decode(&row.sealed.0).map_err(|error| error.to_string())?;
    let seed = old
        .open(&sealed, subject)
        .map_err(|error| error.to_string())?;
    let signer = dialog_credentials::Ed25519Signer::import(&*seed)
        .await
        .map_err(|error| format!("{error:?}"))?;
    if signer.did() != *subject {
        return Err(format!("the custodied seed derives {}", signer.did()));
    }
    let resealed = new
        .seal(&seed, subject)
        .map_err(|error| format!("reseal: {error}"))?
        .encode();
    let replacement = CustodiedSeed::new(subject.clone(), kind, new.did(), resealed);
    reissue(kind, signer, row, replacement).await
}
