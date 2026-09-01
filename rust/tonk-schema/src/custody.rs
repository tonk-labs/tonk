//! Sealed material: a message only its recipient can open, and a
//! principal whose seed one of those messages carries.

use dialog_artifacts::Entity;
use dialog_query::Concept;
use dialog_varsig::Did;
use serde::Serialize;

use crate::domain::custody::{Kind, Message, Seed, Sender, To};
use crate::prelude::*;

/// Ciphertext only its recipient can open.
///
/// The entity is derived from the MESSAGE itself, not from the parties.
/// Sealing is randomized — a fresh ephemeral key and nonce per call — so
/// sealing the same plaintext twice yields different bytes and therefore a
/// different entity. That is deliberate: the row identifies THIS envelope,
/// so a re-seal adds a row rather than silently replacing what it
/// supersedes, and rotation retracts the old one explicitly.
///
/// Deliberately general — nothing here is about accounts or seeds. What a
/// particular envelope CONTAINS is said by whatever points at it: a
/// [`SecretPrincipal`] names its seed this way.
///
/// The bytes are a `tonk_identity::sealed::Sealed` envelope, which binds
/// the recipient and subject DIDs as associated data, so an envelope cannot
/// be re-pointed at another recipient by moving the row.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct SecretMessage {
    /// The row's entity. Derived from the sealed bytes.
    pub this: Entity,
    /// Who can open it: the X25519 `did:key` the bytes were sealed to.
    pub to: To,
    /// The sealed bytes.
    pub message: Message,
    /// Who sealed it, when that is known. Optional because most sealing
    /// has no meaningful sender: a seed sealed to a recipient is just
    /// custody, and naming a sender would invent an author the seal does
    /// not bind. Description, never authority — the seal itself binds
    /// recipient and subject as associated data.
    pub from: Option<Sender>,
}

/// Hash input for [`SecretMessage::this`]. Single-variant enum tags the
/// CBOR encoding with the concept name so equal field data under a
/// different concept hashes differently.
#[derive(Debug, Clone, Serialize)]
enum This<'a> {
    SecretMessage { message: &'a [u8] },
}

impl SecretMessage {
    /// A message sealed to `to`, carrying `message`.
    pub fn new(to: &Did, message: Vec<u8>) -> Self {
        Self {
            this: Entity::of(&This::SecretMessage { message: &message }),
            to: To(to.this()),
            message: Message(message),
            from: None,
        }
    }

    /// The same message, recording who sealed it.
    pub fn sealed_by(mut self, from: &Did) -> Self {
        self.from = Some(Sender(from.this()));
        self
    }

    /// The row's entity.
    pub fn this(&self) -> &Entity {
        &self.this
    }
}

impl AsRef<Entity> for SecretMessage {
    fn as_ref(&self) -> &Entity {
        &self.this
    }
}

/// A principal whose ed25519 seed is held sealed, so it can be re-derived.
///
/// The entity is the principal's own DID. One row per principal, which is
/// the thing being described: `seed` points at the [`SecretMessage`]
/// carrying it, and that message already names its recipient. Sealing the
/// same seed to a second recipient (an admin as recovery custodian) adds a
/// second message and a second row here — the entity carries no recipient,
/// so nothing collides.
///
/// The seed sits beside the ownership delegation rather than on it: the
/// delegation says who may act for the space, the seed says how to re-issue
/// it. Design: `plan/authority-facts.md`, "Wrapped keys".
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct SecretPrincipal {
    /// The principal's DID.
    pub this: Entity,
    /// What this principal is: [`SeedKind::Space`] or [`SeedKind::Invite`].
    pub kind: Kind,
    /// The [`SecretMessage`] whose plaintext is this principal's seed.
    pub seed: Seed,
}

impl SecretPrincipal {
    /// Record that `subject`'s seed is carried by the message at `seed`.
    pub fn new(subject: &Did, kind: SeedKind, seed: &Entity) -> Self {
        Self {
            this: subject.this(),
            kind: kind.kind(),
            seed: Seed(seed.clone()),
        }
    }

    /// The principal's entity.
    pub fn this(&self) -> &Entity {
        &self.this
    }
}

impl AsRef<Entity> for SecretPrincipal {
    fn as_ref(&self) -> &Entity {
        &self.this
    }
}

/// What a sealed seed derives.
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

    /// The kind a stored URI names.
    pub fn parse(uri: &str) -> Option<Self> {
        match uri {
            Self::SPACE => Some(Self::Space),
            Self::INVITE => Some(Self::Invite),
            _ => None,
        }
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

    /// The message's entity is its bytes, so identical ciphertext is one
    /// row and different ciphertext is two — even to the same recipient.
    #[dialog_common::test]
    fn it_keys_a_message_on_its_bytes() {
        let to = did!("test:r");
        let a = SecretMessage::new(&to, vec![1, 2, 3]);
        let b = SecretMessage::new(&to, vec![1, 2, 3]);
        let c = SecretMessage::new(&to, vec![4, 5, 6]);
        assert_eq!(a.this, b.this, "the same envelope is one row");
        assert_ne!(a.this, c.this, "a different envelope is another row");
    }

    /// A principal is keyed on its own DID, so re-sealing its seed to a
    /// new recipient re-points one row rather than adding a second.
    #[dialog_common::test]
    fn it_keys_a_principal_on_its_did() {
        let subject = did!("test:s");
        let first = SecretMessage::new(&did!("test:r1"), vec![1]);
        let second = SecretMessage::new(&did!("test:r2"), vec![2]);
        let a = SecretPrincipal::new(&subject, SeedKind::Space, first.this());
        let b = SecretPrincipal::new(&subject, SeedKind::Space, second.this());
        assert_eq!(a.this, subject.this());
        assert_eq!(a.this, b.this, "one principal is one row");
        assert_ne!(a.seed.0, b.seed.0, "each names its own message");
    }

    /// The pair round-trips: a principal is found by its DID, and the
    /// message it names is found by the entity it points at.
    #[dialog_common::test]
    async fn it_finds_a_principals_seed_through_the_message_it_names() -> Result<()> {
        let (operator, profile) = helpers::test_operator_with_profile().await;
        let repository = helpers::test_repo(&operator, &profile).await;
        let branch = repository.branch("main").open().perform(&operator).await?;
        let subject = did!("test:space");
        let recipient = did!("test:recipient");

        let message = SecretMessage::new(&recipient, vec![9, 9, 9]);
        branch
            .transaction()
            .assert(message.clone())
            .assert(SecretPrincipal::new(
                &subject,
                SeedKind::Space,
                message.this(),
            ))
            .commit()
            .perform(&operator)
            .await?;

        let principals: Vec<SecretPrincipal> = branch
            .query()
            .select(Query::<SecretPrincipal> {
                this: Term::from(subject.this()),
                kind: Term::var("kind"),
                seed: Term::var("seed"),
            })
            .perform(&operator)
            .try_vec()
            .await?;
        assert_eq!(principals.len(), 1);
        assert_eq!(principals[0].kind.0.to_string(), SeedKind::SPACE);

        let messages: Vec<SecretMessage> = branch
            .query()
            .select(Query::<SecretMessage> {
                this: Term::from(principals[0].seed.0.clone()),
                to: Term::var("to"),
                message: Term::var("message"),
                from: Term::var("from"),
            })
            .perform(&operator)
            .try_vec()
            .await?;
        assert_eq!(messages.len(), 1, "the principal names a real message");
        assert_eq!(messages[0].message.0, vec![9, 9, 9]);
        assert_eq!(messages[0].to.0, recipient.this());
        Ok(())
    }

    /// Everything addressed to one recipient is enumerable without
    /// knowing which principals they belong to — the query rotation runs.
    #[dialog_common::test]
    async fn it_lists_every_message_sealed_to_one_recipient() -> Result<()> {
        let (operator, profile) = helpers::test_operator_with_profile().await;
        let repository = helpers::test_repo(&operator, &profile).await;
        let branch = repository.branch("main").open().perform(&operator).await?;
        let mine = did!("test:mine");
        let theirs = did!("test:theirs");

        branch
            .transaction()
            .assert(SecretMessage::new(&mine, vec![1]))
            .assert(SecretMessage::new(&mine, vec![2]))
            .assert(SecretMessage::new(&theirs, vec![3]))
            .commit()
            .perform(&operator)
            .await?;

        let rows: Vec<SecretMessage> = branch
            .query()
            .select(Query::<SecretMessage> {
                this: Term::var("this"),
                to: Term::from(To(mine.this())),
                message: Term::var("message"),
                from: Term::var("from"),
            })
            .perform(&operator)
            .try_vec()
            .await?;

        let mut got: Vec<Vec<u8>> = rows.into_iter().map(|row| row.message.0).collect();
        got.sort();
        assert_eq!(got, vec![vec![1], vec![2]], "only what is sealed to me");
        Ok(())
    }

    /// Sealing the same seed to a second recipient adds a second message
    /// and a second principal row — the recovery-custodian case. The
    /// principal entity carries no recipient, so nothing collides.
    #[dialog_common::test]
    async fn it_seals_one_seed_to_two_recipients() -> Result<()> {
        let (operator, profile) = helpers::test_operator_with_profile().await;
        let repository = helpers::test_repo(&operator, &profile).await;
        let branch = repository.branch("main").open().perform(&operator).await?;
        let subject = did!("test:space");

        let to_me = SecretMessage::new(&did!("test:me"), vec![1]);
        let to_admin = SecretMessage::new(&did!("test:admin"), vec![2]);
        branch
            .transaction()
            .assert(to_me.clone())
            .assert(to_admin.clone())
            .commit()
            .perform(&operator)
            .await?;

        assert_ne!(
            to_me.this, to_admin.this,
            "two seals of one seed are two messages"
        );
        assert_eq!(
            SecretPrincipal::new(&subject, SeedKind::Space, to_me.this()).this,
            SecretPrincipal::new(&subject, SeedKind::Space, to_admin.this()).this,
            "and one principal, whichever message it currently names"
        );
        Ok(())
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

/// Rotate every sealed seed addressed to `old` onto `new`, on `branch`.
///
/// This is the shared rotation core: the worker runs it when a passkey
/// account arrives on a device that onboarded locally, and the CLI runs
/// it at `tonk account login` — one implementation, so the two adapters
/// cannot drift. Per seed the core opens with the old key, derives and
/// verifies the signer, seals to the new recipient, and hands the
/// adapter both the derived signer and the exact replacement rows (the
/// old message to retract, the new message and principal to assert). The
/// adapter re-issues — chains, prefixes, retention, provisioning — and
/// commits the replacement through its own branch handle, because its
/// re-issue writes advance the branch underneath any handle the core
/// could hold.
///
/// Best-effort per seed: a seed that fails to open, verify, or reissue
/// is recorded in [`Rotation::failures`] and left sealed to the old
/// recipient, so a later pass resumes exactly where this one stopped.
pub async fn rotate<Env>(
    branch: &dialog_repository::Branch,
    old: tonk_identity::sealed::AccountSecretKey<'_>,
    new: impl Into<tonk_identity::sealed::AccountSeal>,
    env: &Env,
    mut reissue: impl AsyncFnMut(
        SeedKind,
        dialog_credentials::Ed25519Signer,
        &SecretMessage,
        Replacement,
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
    use dialog_varsig::Principal as _;

    let new = new.into();
    let old_recipient = old.did();

    // Every principal whose seed is carried by a message addressed to the
    // old recipient. Two queries rather than one join: a principal names
    // its message, and the message names who can open it.
    let principals: Vec<SecretPrincipal> = branch
        .query()
        .select(Query::<SecretPrincipal> {
            this: Term::var("this"),
            kind: Term::var("kind"),
            seed: Term::var("seed"),
        })
        .perform(env)
        .try_vec()
        .await
        .map_err(|error| RotateError::Read(format!("{error:?}")))?;
    let messages: Vec<SecretMessage> = branch
        .query()
        .select(Query::<SecretMessage> {
            this: Term::var("this"),
            to: Term::from(To(old_recipient.this())),
            message: Term::var("message"),
            from: Term::var("from"),
        })
        .perform(env)
        .try_vec()
        .await
        .map_err(|error| RotateError::Read(format!("{error:?}")))?;

    let mut rotation = Rotation::default();
    for principal in principals {
        let Some(message) = messages.iter().find(|row| row.this == principal.seed.0) else {
            // Its seed is sealed to someone else; not this pass's work.
            continue;
        };
        let subject: Did = match principal.this.to_string().parse() {
            Ok(subject) => subject,
            Err(error) => {
                rotation.failures.push((
                    old_recipient.clone(),
                    format!("sealed principal is not a DID: {error}"),
                ));
                continue;
            }
        };
        match rotate_seed(old, new, &mut reissue, &subject, &principal, message).await {
            Ok(()) => rotation.rotated.push(subject),
            Err(reason) => rotation.failures.push((subject, reason)),
        }
    }
    Ok(rotation)
}

/// The rows that replace one rotated seed: a message sealed to the new
/// recipient, and the principal naming it.
///
/// Both are asserted together — a principal pointing at a message that was
/// never written would be a seed nothing can open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Replacement {
    /// The re-sealed message.
    pub message: SecretMessage,
    /// The principal, re-pointed at that message.
    pub principal: SecretPrincipal,
}

async fn rotate_seed(
    old: tonk_identity::sealed::AccountSecretKey<'_>,
    new: tonk_identity::sealed::AccountSeal,
    reissue: &mut impl AsyncFnMut(
        SeedKind,
        dialog_credentials::Ed25519Signer,
        &SecretMessage,
        Replacement,
    ) -> Result<(), String>,
    subject: &Did,
    principal: &SecretPrincipal,
    message: &SecretMessage,
) -> Result<(), String> {
    use dialog_varsig::Principal as _;

    let kind = SeedKind::parse(&principal.kind.0.to_string())
        .ok_or_else(|| format!("unknown seed kind {}", principal.kind.0))?;
    let sealed = tonk_identity::sealed::Sealed::decode(&message.message.0)
        .map_err(|error| error.to_string())?;
    let seed = old
        .reveal(&sealed, subject)
        .map_err(|error| error.to_string())?;
    let signer = dialog_credentials::Ed25519Signer::import(&*seed)
        .await
        .map_err(|error| format!("{error:?}"))?;
    if signer.did() != *subject {
        return Err(format!("the sealed seed derives {}", signer.did()));
    }
    let resealed = new
        .conceal(&seed, subject)
        .map_err(|error| format!("reseal: {error}"))?
        .encode();
    let replacement_message = SecretMessage::new(&new.did(), resealed);
    let replacement = Replacement {
        principal: SecretPrincipal::new(subject, kind, replacement_message.this()),
        message: replacement_message,
    };
    reissue(kind, signer, message, replacement).await
}
