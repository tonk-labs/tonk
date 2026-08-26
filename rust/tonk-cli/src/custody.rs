//! Sealing a space's seed to the account — the CLI half of the custody
//! model the worker follows.
//!
//! A space's signing seed must survive this machine: any device on the
//! account can then re-issue the space after a passkey ceremony opens
//! the seed. The account publishes an X25519 recipient
//! ([`AccountEncryptionKey`]) exactly so that sealing needs no passkey —
//! the CLI reads the public half from the account branch it already
//! syncs, seals, and records a [`CustodiedSeed`] row beside the
//! directory entries it writes today. Only a ceremony holding the
//! account secret can ever open the row again.

use anyhow::{Context, Result};
use dialog_operator::Operator;
use dialog_query::{Output as _, Query, Term};
use dialog_repository::Branch;
use dialog_storage::provider::storage::NativeSpace;
use dialog_varsig::Did;
use tonk_identity::sealed::RecipientKey;
use tonk_schema::{AccountEncryptionKey, CustodiedSeed, SeedKind, prelude::DidExt as _};
use zeroize::Zeroizing;

/// The account's published X25519 recipient, read from the account
/// branch. `None` when the account predates the encryption key — a
/// signed-in browser publishes it on its next visit.
pub async fn account_recipient(
    account: &Branch,
    root: &Did,
    operator: &Operator<NativeSpace>,
) -> Result<Option<Did>> {
    let rows: Vec<AccountEncryptionKey> = account
        .query()
        .select(Query::<AccountEncryptionKey> {
            this: Term::from(root.this()),
            key: Term::var("key"),
        })
        .perform(operator)
        .try_vec()
        .await
        .map_err(|error| anyhow::anyhow!("failed to read the account encryption key: {error:?}"))?;
    rows.into_iter()
        .next()
        .map(|row| {
            row.key
                .0
                .to_string()
                .parse()
                .context("the published account encryption key is not a DID")
        })
        .transpose()
}

/// Seal `seed` (deriving `subject`) to `recipient` and record the
/// custody row. The explicit link sequence owns the mandatory account-branch
/// push after this commit.
pub async fn custody_space_seed(
    account: &Branch,
    subject: &Did,
    recipient: &Did,
    seed: &Zeroizing<[u8; 32]>,
    operator: &Operator<NativeSpace>,
) -> Result<()> {
    let key = RecipientKey::from_did(recipient)
        .map_err(|error| anyhow::anyhow!("the account encryption key is unusable: {error}"))?;
    let sealed = key
        .seal(seed, subject)
        .map_err(|error| anyhow::anyhow!("failed to seal the space seed: {error}"))?
        .encode();
    account
        .transaction()
        .assert(CustodiedSeed::new(
            subject.clone(),
            SeedKind::Space,
            recipient.clone(),
            sealed,
        ))
        .commit()
        .perform(operator)
        .await
        .context("failed to record the custodied seed")?;
    Ok(())
}

/// Whether the account branch already holds a decodable custody row for
/// `subject` under the account's current published recipient.
pub async fn has_custody(
    account: &Branch,
    subject: &Did,
    recipient: &Did,
    operator: &Operator<NativeSpace>,
) -> Result<bool> {
    let rows: Vec<CustodiedSeed> = account
        .query()
        .select(Query::<CustodiedSeed> {
            this: Term::var("this"),
            subject: Term::from(tonk_schema::domain::custody::Subject(subject.this())),
            kind: Term::var("kind"),
            recipient: Term::var("recipient"),
            sealed: Term::var("sealed"),
        })
        .perform(operator)
        .try_vec()
        .await
        .map_err(|error| anyhow::anyhow!("failed to read custodied seeds: {error:?}"))?;
    Ok(rows.iter().any(|row| {
        row.recipient.0.to_string() == recipient.to_string()
            && tonk_identity::sealed::Sealed::decode(&row.sealed.0).is_ok()
    }))
}

/// The signing seed a locally created space's stored credential carries.
/// `None` for a space this machine only ever held a verifier for — a
/// joined or delegated space, whose seed is someone else's to custody.
pub async fn site_seed(site: &crate::site::TonkSite) -> Result<Option<Zeroizing<[u8; 32]>>> {
    let Some(signer) = site.repository.credential().signer() else {
        return Ok(None);
    };
    // `Signer` gains arms only when dialog-credentials is built with
    // another algorithm, which this crate never enables.
    let dialog_credentials::Signer::Ed25519(signer) = signer;
    let exported = signer
        .export()
        .await
        .map_err(|error| anyhow::anyhow!("failed to export the space signer: {error:?}"))?;
    let dialog_credentials::KeyExport::Extractable(bytes) = exported;
    let seed: [u8; 32] = bytes
        .as_slice()
        .try_into()
        .context("the exported space seed is not 32 bytes")?;
    Ok(Some(Zeroizing::new(seed)))
}
